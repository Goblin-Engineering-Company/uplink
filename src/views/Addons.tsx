import { useEffect, useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { Catalog, CatalogAddon, Install } from "../lib/types";
import { AddonDot } from "../components/AddonDot";
import { Select } from "../components/Select";
import { versionCompare } from "../lib/version";

// The core Phase-1 surface: pick a WoW install, install/update/uninstall addons,
// control the channel/pin per addon. Roster comes entirely from the catalog.
export function Addons({ ctx }: { ctx: Ctx }) {
  const { config } = ctx;
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [installId, setInstallId] = useState(config.installs[0]?.id ?? "");
  const [expanded, setExpanded] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const load = () => api.reconcileInstalled().then(async (c) => { setCatalog(c); await ctx.reload(); }).catch((e) => setErr(String(e)));
  // Re-run on catalogNonce so a server-side catalog change (heartbeat dirty-stamp)
  // re-pulls the roster without a manual refresh.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  useEffect(() => { load(); }, [ctx.catalogNonce]);

  const install: Install | undefined = config.installs.find((i) => i.id === installId) ?? config.installs[0];

  // ── entitlement + per-addon delivery target (hoisted so the rows AND the
  //    top-of-page "Update all" count share ONE computation) ──
  const siteBase = (config.base_url || "https://goblineng.co").replace(/\/+$/, "");
  const granted = config.account?.channels ?? [];
  const isEntitled = (ch: string) => ch === "public" || granted.includes(ch);
  const entitledChans = catalog?.channel_defs.filter((c) => isEntitled(c.slug)) ?? [];

  // For the SELECTED install: which channel we'd pull `a` from, its latest release,
  // and whether the installed build is behind it. `behind` here is the single source
  // of truth for both a row's "update" chip and the "Update all (N)" button.
  function updateInfo(a: CatalogAddon) {
    const relChannels = Object.keys(a.channels || {}).filter(isEntitled);
    const downloadable = relChannels.length > 0;
    const state = install?.addons.find((x) => x.slug === a.slug);
    const effChan = install ? (state?.channel_override || install.channel) : "public";
    const chan =
      isEntitled(effChan) && a.channels[effChan] ? effChan :
      ([...entitledChans].reverse().find((c) => a.channels[c.slug])?.slug ?? effChan);
    const latest = a.channels[chan] ?? a.channels["public"];
    const behind = !!(downloadable && state?.installed_version && latest &&
      versionCompare(latest.version, state.installed_version) > 0);
    return { downloadable, state, chan, latest, behind };
  }

  // Addons on THIS install with an entitled newer build ready to pull.
  const updatable = (catalog?.addons ?? []).filter((a) => updateInfo(a).behind);

  async function doInstall(a: CatalogAddon, chan: string) {
    if (!install) return;
    const state = install.addons.find((x) => x.slug === a.slug);
    // a pinned version wins over the channel's latest (rollback / hold path)
    const pin = state?.pinned_version ? a.releases.find((r) => r.version === state.pinned_version && r.channel === chan) : undefined;
    const latest = pin ?? a.channels[chan] ?? a.channels["public"];
    if (!latest) { setErr(`No release for ${a.name} on ${chan}`); return; }
    setBusy(a.slug); setErr(null); setNote(null);
    try {
      await api.setAddonSelected(install.id, a.slug, true);
      const msg = await api.installAddon(install.id, a.slug, latest.version, latest.url, chan);
      await api.reportInstalls().catch(() => {});
      setNote(msg);
      await ctx.reload();
    } catch (e) { setErr(String(e)); } finally { setBusy(null); }
  }

  // Update every behind addon on this install in one shot (the Rust `update_all`
  // installs each and advances its recorded version). Surfaces any per-addon FAILURE
  // rather than swallowing it, then reconciles so the count clears.
  async function updateAllNow() {
    if (!install) return;
    setBusy("__all__"); setErr(null); setNote(null);
    try {
      const done = await api.updateAll(install.id);
      await api.reportInstalls().catch(() => {});
      const failed = done.filter((d) => d.includes("FAILED"));
      if (failed.length) setErr(failed.join(" · "));
      setNote(done.length ? `Updated ${done.length - failed.length} of ${done.length}` : "Everything up to date");
      await load(); // reconcile + reload so installed versions and the count refresh
    } catch (e) { setErr(String(e)); } finally { setBusy(null); }
  }

  async function doUninstall(slug: string) {
    if (!install) return;
    setBusy(slug);
    try { await api.uninstallAddon(install.id, slug); await ctx.reload(); }
    catch (e) { setErr(String(e)); } finally { setBusy(null); }
  }

  async function setVersion(slug: string, pinned: string | null, chanOverride: string | null) {
    if (!install) return;
    setBusy(slug); setErr(null); setNote(null);
    try {
      await api.setAddonVersion(install.id, slug, pinned, chanOverride);
      // Apply the switch: move the on-disk addon to the new channel/pin target
      // (up OR down — dev→public downgrades here). SavedVariables are preserved.
      const msgs = await api.reconcileInstall(install.id);
      if (msgs.length) setNote(msgs.join(" · "));
      await ctx.reload();
    } catch (e) { setErr(String(e)); } finally { setBusy(null); }
  }

  if (config.installs.length === 0) return (
    <><div className="page-head"><h1>Addons</h1></div>
      <div className="banner">No WoW installs yet — add one in <b>Settings</b>.</div></>
  );

  return (
    <>
      <div className="page-head"><div className="kicker">Delivery</div><h1>Addons</h1></div>

      <div className="toolbar">
        {config.installs.map((i) => (
          <span key={i.id} className={`pill ${i.id === install?.id ? "active" : ""}`} onClick={() => setInstallId(i.id)}>
            {i.label} · {i.flavor}{i.flavor === "Classic" ? " (untested)" : ""}
            {!i.online && <span className="badge-offline">⚠ unplugged</span>}
          </span>
        ))}
        <span className="spacer" />
        {install?.online && updatable.length > 0 && (
          <button className="btn-spark btn-sm" disabled={!!busy}
            title={`Update ${updatable.length} addon${updatable.length > 1 ? "s" : ""} on ${install.label} · ${install.flavor}`}
            onClick={updateAllNow}>
            {busy === "__all__" ? <span className="spin" /> : `⬆ Update all (${updatable.length})`}
          </button>
        )}
        <button className="btn-ghost btn-sm" onClick={() => { setCatalog(null); load(); }}>↻ Refresh</button>
      </div>

      {install && !install.online && <div className="banner">This install’s path is offline — its config is kept; reconnect the drive to manage addons.</div>}
      {err && <div className="err" style={{ marginBottom: "0.6rem" }}>{err}</div>}
      {note && <div className="ok" style={{ marginBottom: "0.6rem" }}>{note}</div>}

      {catalog === null ? <div className="row"><span className="spin" /> loading catalog…</div> :
        catalog.addons.map((a) => {
          const open = expanded === a.slug;
          // DOWNLOADABLE = there's a build on a channel you're entitled to. THIS —
          // not the raw `status` — decides Download vs Vote: a dev with a dev build
          // downloads a not-yet-public addon; only someone WITHOUT an entitled build
          // sees Vote/Coming-soon. `updateInfo` (hoisted) owns the entitlement + target
          // math so this row and the top "Update all" count never diverge.
          const { downloadable, state, chan, latest, behind } = updateInfo(a);

          if (!downloadable) {
            const votable = a.status === "votable";
            const soon = a.status === "soon" || a.status === "coming_soon";
            const chip = votable ? "Vote" : soon ? "Coming soon" : "Members only";
            return (
              <div key={a.slug} className="panel acc">
                <div className="acc-head" onClick={() => setExpanded(open ? null : a.slug)}>
                  <AddonDot slug={a.slug} size={12} />
                  <b style={{ flex: 1 }}>{a.name}</b>
                  <span className={`chip ${votable ? "chip-spark" : ""}`}>{chip}</span>
                  <span className="faint">{open ? "▲" : "▼"}</span>
                </div>
                {open && (
                  <div className="acc-body">
                    <p>{a.blurb || a.short}</p>
                    <div className="row" style={{ marginTop: "0.4rem" }}>
                      <button className="btn-brass btn-sm" onClick={() => api.openExternal(`${siteBase}/addons/${a.slug}`)}>
                        {votable ? "Vote for this addon ↗" : "Learn more ↗"}
                      </button>
                    </div>
                    <p className="faint" style={{ fontSize: "0.78rem", marginTop: "0.4rem" }}>
                      {votable
                        ? "Not on a channel you can access yet — vote to help it get built. Sign in on the site to vote."
                        : soon
                        ? "On the way. Open its page for details and progress."
                        : "Released on a channel you don't have access to."}
                    </p>
                  </div>
                )}
              </div>
            );
          }

          const chanDef = catalog.channel_defs.find((c) => c.slug === chan);
          const chanLabel = chanDef ? `${chanDef.name}${chanDef.badge ? ` ${chanDef.badge}` : ""}` : chan;
          return (
            <div key={a.slug} className="panel acc">
              <div className="acc-head" onClick={() => setExpanded(open ? null : a.slug)}>
                <AddonDot slug={a.slug} size={12} />
                <b style={{ flex: 1 }}>{a.name}</b>
                <span className="chip chip-brass" title="Delivery channel">{chanLabel}</span>
                <span className="tabular faint">{state?.installed_version ?? "not installed"}</span>
                {behind ? <span className="chip chip-spark">update</span>
                  : state?.installed_version ? <span className="chip chip-green">installed</span>
                  : <span className="chip">available</span>}
                <span className="faint">{open ? "▲" : "▼"}</span>
              </div>
              {open && install && (
                <div className="acc-body">
                  <p>{a.blurb} <a onClick={() => api.openExternal(`${siteBase}/addons/${a.slug}`)} style={{ cursor: "pointer", whiteSpace: "nowrap" }}>View on goblineng.co ↗</a></p>

                  <div className="cog-rule" style={{ margin: "0.5rem 0" }} />
                  <div>
                    <div className="kicker">Version control</div>
                    <div className="stack" style={{ marginTop: "0.4rem" }}>
                      <label className="field"><span>Channel (override install default)</span>
                        <Select value={state?.channel_override ?? ""}
                          options={[{ value: "", label: `Follow install (${install.channel})` },
                            ...entitledChans.map((c) => ({ value: c.slug, label: `${c.name}${c.badge ? ` ${c.badge}` : ""}` }))]}
                          onChange={(v) => setVersion(a.slug, null, v || null)} />
                      </label>
                      <label className="field"><span>Pin version (rollback / hold)</span>
                        <Select value={state?.pinned_version ?? ""}
                          options={[{ value: "", label: "Latest on channel" },
                            ...a.releases.filter((r) => r.channel === chan).map((r) => ({ value: r.version, label: r.version }))]}
                          onChange={(v) => setVersion(a.slug, v || null, state?.channel_override ?? null)} />
                      </label>
                      <label className="check">
                        <input type="checkbox" checked={state?.auto_update ?? false}
                          onChange={(e) => api.setAddonAutoUpdate(install.id, a.slug, e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
                        Auto-update this addon on the schedule
                      </label>
                    </div>
                  </div>

                  <div className="cog-rule" style={{ margin: "0.7rem 0" }} />
                  <div>
                    <div className="kicker">Data sync</div>
                    <label className="check" style={{ marginTop: "0.4rem" }}>
                      <input type="checkbox" checked={state?.sync ?? true}
                        onChange={(e) => api.setAddonSync(install.id, a.slug, e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
                      Sync this addon's data to my account
                    </label>
                    <p className="faint" style={{ margin: "0.2rem 0 0", fontSize: "0.78rem" }}>
                      When on, Uplink uploads the data this addon records for the WoW accounts you picked on the <b>Data</b> page. Turn it off to keep this addon's data local.
                    </p>
                  </div>

                  <div className="cog-rule" style={{ margin: "0.7rem 0" }} />
                  <div className="row">
                    {behind && latest && (
                      <button className="btn-spark btn-sm" disabled={busy === a.slug || !install.online} onClick={() => doInstall(a, chan)}>
                        {busy === a.slug ? <span className="spin" /> : `Update → ${latest.version}`}
                      </button>
                    )}
                    {!state?.installed_version ? (
                      <button className="btn-brass btn-sm" disabled={busy === a.slug || !install.online || !latest} onClick={() => doInstall(a, chan)}>
                        {busy === a.slug ? <span className="spin" /> : latest ? `Install ${latest.version}` : "No release"}
                      </button>
                    ) : (
                      <button className="btn-ghost btn-sm" disabled={busy === a.slug || !install.online} onClick={() => doUninstall(a.slug)}>Uninstall</button>
                    )}
                  </div>
                </div>
              )}
            </div>
          );
        })}
    </>
  );
}
