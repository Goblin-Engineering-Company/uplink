import { Fragment, useEffect, useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { StreamState, SyncLogEntry, SyncTrigger, SyncAccount } from "../lib/types";
import { AddonDot } from "../components/AddonDot";
import { DEFAULT_LANG, formatAgo, secondsSince } from "../lib/i18n";

// Data-sync detail (redesign §5-§7). Reads the cached per install·account·addon·stream
// counters (get_sync_status) and drives a real "Sync now" (sync_now) that sweeps every
// picked account's SavedVariables, uploads to /api/ingest, and advances each cursor from
// the server ack. Status is GROUPED BY ADDON (§5): one quiet row per addon, expandable to
// the per-stream breakdown + the accounts that fed it. Streams are plumbing, not top-level
// rows. Sync while-running + interval settings live here now (§7); base_url stays in Settings.

// Relative time comes from Intl.RelativeTimeFormat via the shared helper, not
// from appending " ago" to a number: word order is per-language ("vor 5 Min.").
// Same helper and same "just now" cutoff as the Home page.
function ago(unix: number | null): string {
  return formatAgo(DEFAULT_LANG, secondsSince(unix), "just now") ?? "never";
}

// What fired a pass, in the user's words. The whole point of the log is that these
// three are told apart: an upload the user asked for reads differently from one that
// happened on its own.
function triggerLabel(t: SyncTrigger): string {
  if (t === "manual") return "Sync now";
  if (t === "instant") return "Instant capture";
  return "Timed check";
}

// One line saying what a pass actually did. A SKIPPED pass is not a failure and must
// not read like one, and "ran and found nothing" is a different answer from "did not
// run at all".
function entrySummary(e: SyncLogEntry): string {
  if (e.error) return e.error;
  if (e.skipped) return `Nothing done, ${e.skipped}`;
  if (e.errors > 0) return `${e.sent} sent, ${e.errors} problem${e.errors === 1 ? "" : "s"}`;
  if (e.sent === 0) return "Nothing new to send";
  return `${e.sent} uploaded, ${e.inserted} new`;
}

// The same answer WITHOUT the counts, for the table, where Uploaded and New are
// already their own columns. Repeating them in a third column just reads as noise.
function entryNote(e: SyncLogEntry): string {
  if (e.error) return e.error;
  if (e.skipped) return `Nothing done, ${e.skipped}`;
  if (e.errors > 0) return `${e.errors} problem${e.errors === 1 ? "" : "s"}`;
  if (e.sent === 0) return "Nothing new to send";
  return "ok";
}

// One stream's rollup across every account, shown only in the expanded detail.
type StreamRow = {
  stream: string;
  queued: number;
  total: number;
  accepted: number;
  lastSync: number | null;
  stuck: boolean;
};

// One ADDON's rollup — the top-level row (§5). Aggregates every stream + every account.
type Row = {
  addon: string;
  accounts: string[];   // real account names contributing (displayed via alias)
  queued: number;
  total: number;
  accepted: number;
  lastSync: number | null;
  stuck: boolean;       // any contributing stream is stuck (queue won't drain)
  streams: StreamRow[];
};

function rollup(states: StreamState[]): Row[] {
  const by = new Map<string, Row>();
  const accts = new Map<string, Set<string>>();       // addon → contributing accounts
  const streams = new Map<string, Map<string, StreamRow>>(); // addon → stream → rollup
  for (const s of states) {
    const r = by.get(s.addon) ?? { addon: s.addon, accounts: [], queued: 0, total: 0, accepted: 0, lastSync: null, stuck: false, streams: [] };
    r.queued += s.queued;
    r.total += s.total;
    r.accepted += s.last_accepted;
    if (s.last_sync && (!r.lastSync || s.last_sync > r.lastSync)) r.lastSync = s.last_sync;
    if (s.stuck && s.queued > 0) r.stuck = true;
    by.set(s.addon, r);

    let as = accts.get(s.addon); if (!as) { as = new Set(); accts.set(s.addon, as); }
    as.add(s.account);

    let sm = streams.get(s.addon); if (!sm) { sm = new Map(); streams.set(s.addon, sm); }
    const sr = sm.get(s.stream) ?? { stream: s.stream, queued: 0, total: 0, accepted: 0, lastSync: null, stuck: false };
    sr.queued += s.queued;
    sr.total += s.total;
    sr.accepted += s.last_accepted;
    if (s.last_sync && (!sr.lastSync || s.last_sync > sr.lastSync)) sr.lastSync = s.last_sync;
    if (s.stuck && s.queued > 0) sr.stuck = true;
    sm.set(s.stream, sr);
  }
  for (const [addon, r] of by) {
    r.accounts = [...(accts.get(addon) ?? [])].sort();
    r.streams = [...(streams.get(addon)?.values() ?? [])].sort((a, b) => a.stream.localeCompare(b.stream));
  }
  return [...by.values()].sort((a, b) => a.addon.localeCompare(b.addon));
}

export function Data({ ctx }: { ctx: Ctx }) {
  const [states, setStates] = useState<StreamState[]>(ctx.config.sync ?? []);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // The sync log: every finished pass, newest first, whatever triggered it. Read from
  // Rust rather than kept from the Sync-now return value, so a pass fired by instant
  // capture or the timed check shows up here too.
  const [log, setLog] = useState<SyncLogEntry[]>([]);
  const [logOpen, setLogOpen] = useState(false);
  // Keyed by the entry itself, not its index: a background pass PREPENDS to the log,
  // so an index would silently move the expanded row onto a different pass.
  const [openEntry, setOpenEntry] = useState<string | null>(null);
  const [accounts, setAccounts] = useState<Record<string, SyncAccount[]>>({});
  const [openAddon, setOpenAddon] = useState<string | null>(null);
  // Inline account-alias editor (§4): which account is being renamed + the draft text.
  const [editingAlias, setEditingAlias] = useState<string | null>(null);
  const [aliasDraft, setAliasDraft] = useState("");

  // Display an account by its alias, falling back to the real folder name. The alias is
  // UI-only — it lets the user hide a real account number/name in screen recordings; the
  // real name stays the sync/wire key everywhere in Rust.
  const aliasOf = (acct: string) => ctx.config.account_aliases?.[acct] ?? acct;

  // Key the discovery effect on CONTENT, not array identity: every ctx.reload() reads
  // the config fresh from Rust, so `installs` is a brand-new array object each time and
  // an identity dep re-fires the whole 1+N invoke burst on every heartbeat / focus /
  // checkbox toggle (list_sync_accounts also rewrites the config file per call).
  const onlineKey = ctx.config.installs.filter((i) => i.online).map((i) => i.id).join(",");

  useEffect(() => {
    let alive = true;
    api.getSyncStatus().then((s) => { if (alive) setStates(s); }).catch((e) => { if (alive) setErr(String(e)); });
    // ctx.syncNonce bumps on every uplink:sync-finished, so a background pass refreshes
    // the log the same way it refreshes the counters.
    api.getSyncLog().then((l) => { if (alive) setLog(l); }).catch(() => {});
    // discover + resolve selected accounts for each online install
    for (const id of onlineKey ? onlineKey.split(",") : []) {
      api.listSyncAccounts(id)
        .then((a) => { if (alive) setAccounts((prev) => ({ ...prev, [id]: a })); })
        .catch(() => {});
    }
    return () => { alive = false; };
  }, [onlineKey, ctx.syncNonce]);

  async function toggleAccount(installId: string, account: string, checked: boolean) {
    const cur = accounts[installId] ?? [];
    const selected = cur.filter((a) => a.selected).map((a) => a.account);
    const next = checked
      ? [...new Set([...selected, account])]
      : selected.filter((a) => a !== account);
    await api.setSyncAccounts(installId, next);
    const fresh = await api.listSyncAccounts(installId).catch(() => cur);
    setAccounts((prev) => ({ ...prev, [installId]: fresh }));
    await ctx.reload();
  }

  async function saveAlias(account: string) {
    await api.setAccountAlias(account, aliasDraft.trim());
    setEditingAlias(null);
    await ctx.reload();
  }

  const rows = rollup(states);
  const totalQueued = rows.reduce((n, r) => n + r.queued, 0);
  // Stuck addons (§6): a queue that won't drain (renamed/removed stream, server-unknown).
  // "Clear queue" surfaces ONLY when something is genuinely stuck — otherwise it's hidden.
  const stuckRows = rows.filter((r) => r.stuck);
  const anyStuck = stuckRows.length > 0;

  async function syncNow() {
    setBusy(true);
    setErr(null);
    try {
      await api.syncNow();
      setStates(await api.getSyncStatus());
      setLog(await api.getSyncLog().catch(() => log));
      await ctx.reload();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function fullResync() {
    // Plain-language confirm: no jargon. The server removes duplicates, so this is
    // always safe — it just re-sends everything from the beginning.
    const ok = await api.confirm(
      "This uploads everything again from the start. It's safe: the site removes duplicates automatically, so nothing gets counted twice. Handy if something looks missing or wrong.",
      "Re-send all your data?"
    );
    if (!ok) return;
    setBusy(true);
    setErr(null);
    try {
      await api.resetCursors();
      await api.syncNow();
      setStates(await api.getSyncStatus());
      setLog(await api.getSyncLog().catch(() => log));
      await ctx.reload();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  // Purge the queue: forget all local sync state so a stuck/orphaned "waiting" count clears.
  // The usual cause is a stream that was renamed or removed in an addon update (its old queue
  // can never drain). Live data re-discovers and re-sends on the next sync; the site removes
  // duplicates, so nothing is lost or double-counted.
  async function purgeQueue() {
    const ok = await api.confirm(
      "This clears the local upload queue. Use it when something stays stuck on \"waiting\" and won't send. " +
      "Your uploaded data on the site is untouched, and current data re-sends on the next sync (duplicates are removed). Clear it?",
      "Clear the upload queue?"
    );
    if (!ok) return;
    setBusy(true);
    setErr(null);
    try {
      await api.purgeSyncQueue();
      setStates(await api.getSyncStatus());
      setLog(await api.getSyncLog().catch(() => log));
      await ctx.reload();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <div className="page-head"><div className="kicker">Data sync</div><h1>Data</h1></div>

      {/* ── Per-install WoW-account selection ── */}
      <div className="panel card">
        <h3 style={{ margin: 0 }}>WoW accounts to sync</h3>
        <p className="muted" style={{ margin: "0.4rem 0 0", fontSize: "0.85rem" }}>
          Pick the accounts you want to sync. Only choose accounts that are <b>yours</b>, because whatever you
          pick gets uploaded to your account. Use <b>Rename</b> to show a friendlier name (your real account name
          stays private, handy for screen recordings).
        </p>
        <div className="stack" style={{ marginTop: "0.7rem" }}>
          {ctx.config.installs.length === 0 && <p className="muted" style={{ margin: 0 }}>No installs configured.</p>}
          {ctx.config.installs.map((inst) => {
            const accts = accounts[inst.id] ?? [];
            return (
              <div key={inst.id} className="panel" style={{ padding: "0.7rem 1rem" }}>
                <div className="row between">
                  <div><b>{inst.label}</b> <span className="chip">{inst.flavor}</span> {!inst.online && <span className="badge-offline">⚠ unplugged</span>}</div>
                </div>
                {!inst.online ? (
                  <div className="faint" style={{ fontSize: "0.8rem", marginTop: 4 }}>This game folder isn’t connected right now.</div>
                ) : accts.length === 0 ? (
                  <div className="faint" style={{ fontSize: "0.8rem", marginTop: 4 }}>No accounts found yet. Log into the game once, then check back.</div>
                ) : (
                  <div className="stack" style={{ marginTop: "0.5rem", gap: "0.3rem" }}>
                    {accts.map((a) => (
                      <div key={a.account} className="row between">
                        <label className="check">
                          <input type="checkbox" checked={a.selected}
                            onChange={(e) => toggleAccount(inst.id, a.account, e.target.checked)} />
                          <span className="tabular">{aliasOf(a.account)}</span>
                          {a.selected && <span className="chip chip-brass" style={{ marginLeft: 6 }}>syncing</span>}
                        </label>
                        {editingAlias === a.account ? (
                          <span className="row" style={{ gap: 4 }}>
                            <input value={aliasDraft} autoFocus onChange={(e) => setAliasDraft(e.target.value)}
                              placeholder={a.account} style={{ maxWidth: 160 }}
                              onKeyDown={(e) => { if (e.key === "Enter") saveAlias(a.account); if (e.key === "Escape") setEditingAlias(null); }} />
                            <button className="btn-brass btn-sm" onClick={() => saveAlias(a.account)}>Save</button>
                            <button className="btn-ghost btn-sm" onClick={() => setEditingAlias(null)}>Cancel</button>
                          </span>
                        ) : (
                          <button className="btn-ghost btn-sm"
                            title="Rename how this account is shown, so your real account name stays out of screen recordings"
                            onClick={() => { setEditingAlias(a.account); setAliasDraft(ctx.config.account_aliases?.[a.account] ?? ""); }}>
                            Rename
                          </button>
                        )}
                      </div>
                    ))}
                    {accts.every((a) => !a.selected) && (
                      <div className="faint" style={{ fontSize: "0.78rem" }}>Nothing picked, so nothing will sync here.</div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      <div className="panel card">
        <div className="row between">
          <h3 style={{ margin: 0 }}>Sync status {totalQueued > 0 && <span className="chip chip-brass">{totalQueued} waiting</span>}</h3>
          <div className="row" style={{ gap: "0.5rem" }}>
            {/* Clear queue is hidden unless a stream is genuinely stuck (§6). */}
            {anyStuck && (
              <button className="btn-ghost btn-sm" disabled={busy} onClick={purgeQueue}
                      title="A data stream is stuck and won't send (often after an addon update renamed it). Clear the stuck queue.">
                Clear queue
              </button>
            )}
            <button className="btn-ghost btn-sm" disabled={busy || !ctx.config.paired} onClick={fullResync}
                    title={ctx.config.paired ? "Re-send everything from the start (safe; duplicates are removed)" : "Pair this device first"}>
              Full resync
            </button>
            <button className="btn-brass btn-sm" disabled={busy || !ctx.config.paired} onClick={syncNow}
                    title={ctx.config.paired ? "Upload now" : "Pair this device first"}>
              {busy ? <span className="spin" /> : "Sync now"}
            </button>
          </div>
        </div>

        {/* Stuck indicator (§6): name the addon · stream that won't drain. */}
        {anyStuck && (
          <div className="banner err" style={{ marginTop: "0.6rem" }}>
            ⚠ Stuck queue. This won't send on its own; click <b>Clear queue</b> to reset it.
            <div className="faint" style={{ fontSize: "0.8rem", marginTop: 4 }}>
              {stuckRows.map((r) => `${r.addon} · ${r.streams.filter((s) => s.stuck).map((s) => s.stream).join(", ")}`).join("  •  ")}
            </div>
          </div>
        )}

        <p className="muted" style={{ margin: "0.6rem 0 0", fontSize: "0.8rem" }}>
          <b>Full resync</b> re-sends all your data from the beginning. It’s safe: the site removes duplicates,
          so nothing is counted twice. Click a row to see each data stream and the accounts it came from.
        </p>

        <table style={{ width: "100%", marginTop: "0.8rem", borderCollapse: "collapse" }} className="tabular">
          <thead>
            <tr style={{ textAlign: "left", color: "var(--color-ink-muted)", fontSize: "0.75rem" }}>
              <th style={{ padding: "0.4rem 0" }}>Addon</th><th>Accounts</th>
              <th>Last upload</th><th>Uploaded</th><th>Waiting</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && !err && <tr><td colSpan={5} className="muted" style={{ padding: "0.6rem 0" }}>
              Nothing synced yet. Click <b>Sync now</b> to upload your data.</td></tr>}
            {rows.length === 0 && err && <tr><td colSpan={5} className="faint" style={{ padding: "0.6rem 0" }}>
              Couldn’t load sync status. The error is below.</td></tr>}
            {rows.map((r) => {
              const open = openAddon === r.addon;
              return (
                <Fragment key={r.addon}>
                  <tr style={{ borderTop: "1px solid var(--color-line)", cursor: "pointer" }}
                      onClick={() => setOpenAddon(open ? null : r.addon)}>
                    <td style={{ padding: "0.5rem 0" }}>
                      <span className="row" style={{ gap: "0.4rem" }}>
                        <span className="faint" style={{ width: "1em" }}>{open ? "▲" : "▼"}</span>
                        <AddonDot slug={r.addon} /> {r.addon}
                        {r.stuck && <span className="chip chip-spark" title="A data stream is stuck">stuck</span>}
                      </span>
                    </td>
                    <td className="faint">{r.accounts.length}</td>
                    <td className="faint">{ago(r.lastSync)}</td>
                    <td>{r.total}</td>
                    <td className={r.queued > 0 ? "" : "faint"}>
                      {r.queued > 0 ? <span className="chip chip-brass">{r.queued} waiting</span> : 0}
                    </td>
                  </tr>
                  {open && (
                    <tr style={{ background: "var(--color-panel-2, rgba(0,0,0,0.12))" }}>
                      <td colSpan={5} style={{ padding: "0.4rem 0 0.7rem 1.6rem" }}>
                        <div className="faint" style={{ fontSize: "0.75rem", margin: "0.2rem 0 0.3rem" }}>
                          Accounts: {r.accounts.length ? r.accounts.map(aliasOf).join(", ") : "—"}
                        </div>
                        <table style={{ width: "100%", borderCollapse: "collapse" }} className="tabular">
                          <thead>
                            <tr style={{ textAlign: "left", color: "var(--color-ink-muted)", fontSize: "0.72rem" }}>
                              <th style={{ padding: "0.2rem 0" }}>Data stream</th><th>Last upload</th><th>Uploaded</th><th>Waiting</th>
                            </tr>
                          </thead>
                          <tbody>
                            {r.streams.map((s) => (
                              <tr key={s.stream}>
                                <td style={{ padding: "0.2rem 0" }}>
                                  {s.stream} {s.stuck && <span className="chip chip-spark" title="Stuck and won't send">stuck</span>}
                                </td>
                                <td className="faint">{ago(s.lastSync)}</td>
                                <td>{s.total}</td>
                                <td className={s.queued > 0 ? "" : "faint"}>{s.queued}</td>
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </td>
                    </tr>
                  )}
                </Fragment>
              );
            })}
          </tbody>
        </table>

        {/* ── Sync log ── Right under the status table it describes, not at the bottom
            of the page: the user presses "Sync now" up there and has to see what it did.
            Collapsed to one line by default (same expander idiom as the addon rows), so
            it is always present without taking the page over. Fed by EVERY pass, not
            just the button. ── */}
        <div className="cog-rule" style={{ margin: "0.9rem 0 0.6rem" }} />
        <div className="row between" style={{ cursor: "pointer" }}
             onClick={() => setLogOpen(!logOpen)}
             title="Every sync, including the ones that run on their own">
          <h3 style={{ margin: 0 }}>
            <span className="faint" style={{ marginRight: "0.4rem" }}>{logOpen ? "▲" : "▼"}</span>
            Sync log
          </h3>
          {log.length > 0 ? (
            <span className="faint" style={{ fontSize: "0.8rem" }}>
              {triggerLabel(log[0].trigger)} · {ago(log[0].at)} · {entrySummary(log[0])}
            </span>
          ) : (
            <span className="faint" style={{ fontSize: "0.8rem" }}>Nothing recorded yet</span>
          )}
        </div>

        {logOpen && (
          <table style={{ width: "100%", marginTop: "0.5rem", borderCollapse: "collapse" }} className="tabular">
            <thead>
              <tr style={{ textAlign: "left", color: "var(--color-ink-muted)", fontSize: "0.75rem" }}>
                <th style={{ padding: "0.4rem 0" }}>When</th><th>What started it</th>
                <th title="Records Uplink uploaded in this pass. Only what was waiting at the time, so it is not a running total.">Uploaded</th>
                <th title="How many of those the site had not seen before. The rest were duplicates and were discarded.">New</th>
                <th>Note</th>
              </tr>
            </thead>
            <tbody>
              {log.length === 0 && <tr><td colSpan={5} className="muted" style={{ padding: "0.6rem 0" }}>
                No syncs yet. This fills in as your data uploads, whether you press Sync now or Uplink does it for you.</td></tr>}
              {log.map((e, i) => {
                const key = `${e.at}-${e.trigger}`;
                const open = openEntry === key;
                return (
                  <Fragment key={`${key}-${i}`}>
                    <tr style={{ borderTop: "1px solid var(--color-line)", cursor: e.results.length ? "pointer" : "default" }}
                        onClick={() => setOpenEntry(open ? null : e.results.length ? key : null)}>
                      <td style={{ padding: "0.4rem 0" }}>
                        <span className="faint" style={{ marginRight: "0.4rem" }}>
                          {e.results.length ? (open ? "▲" : "▼") : " "}
                        </span>
                        {ago(e.at)}
                      </td>
                      <td>{triggerLabel(e.trigger)}</td>
                      <td className="faint">{e.sent}</td>
                      <td>{e.inserted}</td>
                      <td className={e.error || e.errors > 0 ? "err" : "faint"}>{entryNote(e)}</td>
                    </tr>
                    {open && (
                      <tr style={{ background: "var(--color-panel-2, rgba(0,0,0,0.12))" }}>
                        <td colSpan={5} style={{ padding: "0.4rem 0 0.7rem 1.6rem" }}>
                          <table style={{ width: "100%", borderCollapse: "collapse" }} className="tabular">
                            <thead>
                              <tr style={{ textAlign: "left", color: "var(--color-ink-muted)", fontSize: "0.72rem" }}>
                                <th style={{ padding: "0.2rem 0" }}>Account</th><th>What</th><th title="Records uploaded from this stream in this pass.">Uploaded</th><th title="How many of those the site had not seen before.">New</th><th>Note</th>
                              </tr>
                            </thead>
                            <tbody>
                              {e.results.map((r, j) => (
                                <tr key={j}>
                                  <td style={{ padding: "0.2rem 0" }}>{r.skipped && !r.install_label ? "—" : `${r.install_label} · ${aliasOf(r.account)}`}</td>
                                  <td>{r.addon ? `${r.addon}${r.stream ? ` · ${r.stream}` : ""}` : "—"}</td>
                                  <td className="faint">{r.found}</td>
                                  <td title="Rows the site newly stored (duplicates skipped)">{r.inserted}</td>
                                  <td className={r.error ? "err" : r.warning ? "" : "faint"}>
                                    {r.error ? r.error
                                      : r.warning ? <span title="Accepted, but flagged by the site">⚠ {r.warning}</span>
                                      : r.unknown ? "not used by the site yet"
                                      : r.skipped ? r.skipped : "ok"}
                                  </td>
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}

        {/* ── Data capture (sync options; here rather than Settings, redesign §7) ──
            Order is deliberate: instant capture is the primary path, the timed check
            reads as the fallback BEHIND it. No coupling between the two boxes — the
            watcher fires as `manual = true` and bypasses the while-running gate by
            design, so instant capture never depends on the second box. ── */}
        <div className="cog-rule" style={{ margin: "0.9rem 0 0.7rem" }} />
        <label className="check">
          <input type="checkbox" checked={ctx.config.watch_enabled} onChange={(e) => api.setWatchEnabled(e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
          Instant capture <span className="faint">(recommended)</span>
        </label>
        <p className="faint" style={{ margin: "0.2rem 0 0.7rem", fontSize: "0.78rem" }}>
          Uplink watches your addons' saved data and uploads it seconds after the game writes it. Type <b>/reload</b> in game
          and it's on the server before you've finished your next cast. This runs even while the game is open, so it ignores
          the timer setting below.
        </p>
        <label className="check">
          <input type="checkbox" checked={ctx.config.sync_while_running} onChange={(e) => api.setSyncWhileRunning(e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
          Timed check while the game is open <span className="faint">(fallback)</span>
        </label>
        <p className="faint" style={{ margin: "0.2rem 0 0.7rem", fontSize: "0.78rem" }}>
          Best left on. Instant capture does not need it, but it is what catches your data if instant capture cannot start,
          which happens when the file watch is blocked or the drive holding your install is unplugged. Uplink notes that in
          the log when it happens. While instant capture is working this rarely finds anything, since WoW only writes its
          saved data on <b>/reload</b> or logout. Turn it off if you want nothing touching the disk while you play.
        </p>
        <div className="grid2">
          <label className="field"><span>Timed check every (seconds)</span>
            <input type="number" min={30} defaultValue={ctx.config.sync_interval_secs}
              onBlur={(e) => { const n = Math.max(30, parseInt(e.target.value || "0", 10) || 0); api.setSyncInterval(n).then(ctx.reload).catch(() => ctx.reload()); }} />
          </label>
          <label className="field"><span>Settle time before instant capture <span className="faint">(advanced)</span></span>
            <div className="row">
              <input type="number" min={100} defaultValue={ctx.config.watch_debounce_ms}
                onBlur={(e) => { const n = Math.max(100, parseInt(e.target.value || "0", 10) || 0); api.setWatchDebounce(n).then(ctx.reload).catch(() => ctx.reload()); }} />
              <span className="faint">ms</span>
            </div>
          </label>
        </div>
        <p className="faint" style={{ margin: "0.4rem 0 0", fontSize: "0.78rem" }}>
          Uplink also checks when you open it and after you log out of the game. Settle time is how long Uplink waits for the
          game to finish writing before it reads the files; raise it if a slow drive gives you half-written uploads.
        </p>
      </div>

      {err && <div className="banner err" style={{ marginTop: "0.8rem" }}>{err}</div>}
    </>
  );
}
