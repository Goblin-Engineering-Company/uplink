import { useEffect, useRef, useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { CatalogAddon, StreamState, UpdateItem } from "../lib/types";
import { AddonDot } from "../components/AddonDot";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { getVersion } from "@tauri-apps/api/app";
import gecLogo from "../assets/gec-logo.png";

// The collapsed daily view that drops down from the menu-bar / tray icon.
// Layout mirrors spec §4.1. Per-addon sync state (✓ synced / ⧗ N queued / — idle)
// rolls up the get_sync_status counters across accounts.
export function TrayPanel({ ctx }: { ctx: Ctx }) {
  const { config } = ctx;
  const [updates, setUpdates] = useState<UpdateItem[]>([]);
  const [catalog, setCatalog] = useState<CatalogAddon[]>([]);
  const [sync, setSync] = useState<StreamState[]>(config.sync ?? []);
  const [busy, setBusy] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [appVersion, setAppVersion] = useState("");
  const [selfUpdate, setSelfUpdate] = useState<string | null>(null);
  const [selfBusy, setSelfBusy] = useState(false);
  // Outcome of the last bulk run. The tray used to swallow every failure with
  // `.catch(() => [])`, so a failed update and a successful no-op looked identical —
  // which is how UP-005 stayed invisible. `bad` picks the error styling.
  const [note, setNote] = useState<{ text: string; bad: boolean } | null>(null);
  const bodyRef = useRef<HTMLDivElement>(null);

  // Pull fresh catalog / updates / sync. Runs on open AND every time the panel regains focus, so a purge
  // or update done in the main window (a SEPARATE webview) is reflected here immediately instead of going
  // stale. Also re-reads config (roster) so the addon list tracks what's actually installed.
  async function refresh() {
    await ctx.reload().catch(() => {});
    const [c, u, s, su] = await Promise.all([
      api.fetchCatalog().then((r) => r.addons).catch(() => null),
      api.checkUpdates().catch(() => null),
      api.getSyncStatus().catch(() => null),
      // Uplink's own update belongs in the Updates section too (Flatpak builds
      // can't self-update, so don't even ask there).
      config.is_flatpak ? Promise.resolve(null) : api.checkSelfUpdate().catch(() => null),
    ]);
    if (c) setCatalog(c);
    if (u) setUpdates(u);
    if (s) setSync(s);
    setSelfUpdate(su && su.kind === "available" ? su.version : null);
  }

  useEffect(() => {
    let alive = true;
    // Kick these off the effect's synchronous body: they are async loads FROM an
    // external system (Tauri IPC), not React state being synchronised into one.
    (async () => {
      const v = await getVersion().catch(() => "");
      if (alive) setAppVersion(v);
      await refresh();
    })();
    const w = getCurrentWebviewWindow();
    const un = w.onFocusChanged(({ payload: focused }) => { if (focused) refresh(); });
    return () => { alive = false; un.then((f) => f()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // roster: only addons that actually SYNC — those that have produced sync streams.
  // Built from the sync status (like the Data page), NOT the installed-addon list,
  // so non-data addons (e.g. GEC-Console) never appear here. An addon shows up once
  // it has a stream; "nothing synced yet" shows the empty note below.
  const roster = new Map<string, { slug: string; name: string }>();
  for (const s of sync) {
    if (roster.has(s.addon)) continue;
    const def = catalog.find((c) => c.slug === s.addon);
    roster.set(s.addon, { slug: s.addon, name: def?.name ?? s.addon });
  }

  // per-addon roll-up of sync counters (across accounts/streams)
  function syncFor(slug: string) {
    const rows = sync.filter((s) => s.addon === slug);
    const queued = rows.reduce((n, s) => n + s.queued, 0);
    const total = rows.reduce((n, s) => n + s.total, 0);
    const last = rows.reduce<number | null>((m, s) => (s.last_sync && (!m || s.last_sync > m) ? s.last_sync : m), null);
    return { queued, total, last, seen: rows.length > 0 };
  }
  function stateText(slug: string): string {
    const { queued, total, last, seen } = syncFor(slug);
    if (queued > 0) return `⧗ ${queued} queued`;
    if (last) return `✓ ${total} synced`;
    if (seen && total > 0) return `${total} entries`;
    return "— idle";
  }
  const totalQueued = [...roster.keys()].reduce((n, slug) => n + syncFor(slug).queued, 0);
  // an update belongs to a specific install → show WHICH game flavor it's for (Retail / Classic / …), since
  // the same addon can be installed in several flavors at different versions.
  const flavorOf = (installId: string) => config.installs.find((i) => i.id === installId)?.flavor ?? "";
  // `updates` comes from Rust `compute_updates` — the same list `update_all` executes —
  // so this count and the bulk action can't disagree. Rows that never landed are first
  // installs, not version bumps; label the button for what it will actually do.
  const verb = updates.length > 0 && updates.every((u) => u.missing) ? "Install" : "Update";

  // Size the tray window to fit its content (up to a cap) so the Update / Sync buttons are never below a
  // scroll. Re-measures whenever the visible content changes. Preserves the current width; clamps height.
  // Measures the body's inner wrapper, NOT .tray — .tray is stretched to 100% height, so its scrollHeight
  // can never drop below the current window and the panel used to grow but never shrink back (the big
  // empty gap above the footer once updates/sync lines went away).
  // Declared HERE (below the derived values) because its dep list names them; each dep is a piece of
  // content that adds or removes a row/button, i.e. changes the natural height. Without a dep list this
  // ran after every render — three IPC round-trips each — which is ~18 calls and visible jitter on open.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const inner = bodyRef.current;
      const body = inner?.parentElement;                              // .tray-body (flex-stretched)
      const tray = inner?.closest(".tray") as HTMLElement | null;
      if (!inner || !body || !tray) return;
      const w = getCurrentWebviewWindow();
      try {
        const factor = await w.scaleFactor();
        const cur = await w.innerSize();
        const widthLogical = Math.round(cur.width / factor);
        const cs = getComputedStyle(body);
        const padY = (parseFloat(cs.paddingTop) || 0) + (parseFloat(cs.paddingBottom) || 0);
        const chrome = tray.scrollHeight - body.clientHeight;         // head + hairline + foot
        const natural = chrome + inner.offsetHeight + padY;
        const h = Math.min(720, Math.max(120, Math.ceil(natural) + 2));
        if (!cancelled) await w.setSize(new LogicalSize(widthLogical, h));
      } catch { /* window sizing best-effort; content still scrolls if it fails */ }
    })();
    return () => { cancelled = true; };
  }, [roster.size, updates.length, selfUpdate, syncing, busy, selfBusy, note, config.paired, config.installs.length]);

  async function updateAll() {
    setBusy(true); setNote(null);
    const done: string[] = [];
    const errors: string[] = [];
    try {
      for (const inst of config.installs) {
        try { done.push(...(await api.updateAll(inst.id))); }
        catch (e) { errors.push(`${inst.label}: ${String(e)}`); }
      }
      const failed = done.filter((d) => d.includes("FAILED"));
      const bad = [...errors, ...failed];
      setNote(bad.length
        ? { text: bad.slice(0, 2).join(" · "), bad: true }
        : done.length
        ? { text: `${verb}d ${done.length}`, bad: false }
        // The button offered work and none was done: say it instead of looking successful.
        : { text: updates.length ? "Nothing was installed. Refreshed the list" : "Nothing to update", bad: updates.length > 0 });
      await api.reportInstalls().catch(() => {});
      setUpdates(await api.checkUpdates().catch(() => []));
      await ctx.reload();
    } finally { setBusy(false); }
  }

  async function syncNow() {
    setSyncing(true);
    try {
      await api.syncNow().catch(() => []);
      setSync(await api.getSyncStatus().catch(() => []));
      await ctx.reload();
    } finally { setSyncing(false); }
  }

  async function updateSelf() {
    setSelfBusy(true);
    // relaunches on success, so it may never resolve; only clear the spinner on failure
    api.installSelfUpdate().catch(() => setSelfBusy(false));
  }

  async function openMain() {
    const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
    const w = await WebviewWindow.getByLabel("main");
    if (w) { await w.show(); await w.setFocus(); }
  }

  return (
    <div className="tray">
      <div className="hairline" />
      <div className="tray-head">
        <img src={gecLogo} className="brandmark-logo" style={{ width: 24, height: 24 }} alt="GEC" />
        <b className="display brass-text">Uplink</b>
        <span className={`dot ${config.paired ? "dot-on" : "dot-off"}`} title={config.paired ? "connected" : "not paired"} />
        <span className="spacer" />
        {config.account && <span className="chip chip-brass" title={`${config.account.role}${config.account.tier ? " · " + config.account.tier : ""}`}>{config.account.handle || config.device_name || "device"}</span>}
      </div>

      <div className="tray-body">
        <div ref={bodyRef}>
        <div className="kicker" style={{ marginBottom: "0.3rem" }}>Sync by addon</div>
        {roster.size === 0 && <div className="muted" style={{ fontSize: "0.85rem" }}>Nothing synced yet.</div>}
        {[...roster.values()].map((r) => (
          <div key={r.slug} className="sync-line">
            <AddonDot slug={r.slug} />
            <span className="name">{r.name}</span>
            <span className="count">{stateText(r.slug)}</span>
          </div>
        ))}
        {config.paired && config.installs.length > 0 && (
          <button className="btn-brass btn-sm" style={{ width: "100%", marginTop: "0.5rem" }} disabled={syncing} onClick={syncNow}>
            {syncing ? <span className="spin" /> : totalQueued > 0 ? `Sync now (${totalQueued})` : "Sync now"}
          </button>
        )}

        {(updates.length > 0 || selfUpdate) && (
          <>
            <div className="cog-rule" style={{ margin: "0.6rem 0" }} />
            <div className="kicker" style={{ marginBottom: "0.3rem" }}>Updates</div>
            {selfUpdate && (
              <div className="sync-line">
                <img src={gecLogo} style={{ width: 12, height: 12 }} alt="" />
                <span className="name">
                  Uplink
                  {" "}<span className="faint tabular">{appVersion || "—"} → {selfUpdate}</span>
                </span>
              </div>
            )}
            {updates.map((u) => (
              <div key={u.install_id + u.slug} className="sync-line">
                <AddonDot slug={u.slug} />
                <span className="name">
                  {u.slug}
                  {flavorOf(u.install_id) && <span className="chip" style={{ marginLeft: 5, fontSize: "0.68rem" }}>{flavorOf(u.install_id)}</span>}
                  {" "}<span className="faint tabular">{u.installed ?? "—"} → {u.latest}</span>
                </span>
              </div>
            ))}
            {selfUpdate && (
              <button className="btn-spark btn-sm" style={{ width: "100%", marginTop: "0.5rem" }} disabled={selfBusy} onClick={updateSelf}>
                {selfBusy ? <span className="spin" /> : "Update Uplink & restart"}
              </button>
            )}
            {updates.length > 0 && (
              <button className="btn-spark btn-sm" style={{ width: "100%", marginTop: "0.5rem" }} disabled={busy} onClick={updateAll}>
                {busy ? <span className="spin" /> : `${verb} all (${updates.length})`}
              </button>
            )}
          </>
        )}
        {note && (
          <div className={note.bad ? "err" : "ok"} style={{ marginTop: "0.5rem", fontSize: "0.78rem" }}>{note.text}</div>
        )}
        </div>
      </div>

      <div className="tray-foot">
        <button className="btn-brass btn-sm" style={{ flex: 1 }} onClick={openMain}>Open Uplink</button>
      </div>
    </div>
  );
}
