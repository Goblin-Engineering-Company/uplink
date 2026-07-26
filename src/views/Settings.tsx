import { useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import { Select } from "../components/Select";
import type { Catalog } from "../lib/types";
import { useEffect } from "react";
import { UpdatePanel } from "../components/UpdatePanel";

export function Settings({ ctx }: { ctx: Ctx }) {
  const { config } = ctx;
  const [baseUrl, setBaseUrl] = useState(config.base_url);
  const [savedNote, setSavedNote] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [pairCode, setPairCode] = useState("");
  const [err, setErr] = useState<string | null>(null);
  // Launch-at-login state is OS-owned (not in config) — fetch it live on mount.
  const [launchAtLogin, setLaunchAtLogin] = useState(false);

  // ctx.catalogNonce: re-fetch when the heartbeat's catalog dirty-stamp moves.
  useEffect(() => { api.fetchCatalog().then(setCatalog).catch(() => setCatalog({ schema: "", channel_defs: [{ slug: "public", name: "Public", badge: "", sort: 0 }], addons: [] })); }, [ctx.catalogNonce]);
  useEffect(() => { api.getLaunchAtLogin().then(setLaunchAtLogin).catch(() => {}); }, []);

  async function toggleLaunchAtLogin(enabled: boolean) {
    await api.setLaunchAtLogin(enabled);
    api.getLaunchAtLogin().then(setLaunchAtLogin).catch(() => {});
  }

  async function saveBaseUrl() {
    await api.setBaseUrl(baseUrl.trim());
    await ctx.reload();
    setSavedNote("Base URL saved"); setTimeout(() => setSavedNote(null), 2000);
    setCatalog(null); api.fetchCatalog().then(setCatalog).catch(() => {});
  }

  async function pair() {
    setErr(null);
    try { await api.pairDevice(pairCode.trim().toUpperCase()); setPairCode(""); await ctx.reload(); }
    catch (e) { setErr(String(e)); }
  }

  async function browseAdd() {
    const p = await api.browseFolder();
    if (p && typeof p === "string") { await api.addInstall(p, ""); await ctx.reload(); }
  }


  const channels = catalog?.channel_defs ?? [{ slug: "public", name: "Public", badge: "", sort: 0 }];
  const granted = config.account?.channels ?? [];
  const entitled = channels.filter((c) => c.slug === "public" || granted.includes(c.slug));

  return (
    <>
      <div className="page-head"><div className="kicker">Preferences</div><h1>Settings</h1></div>

      {/* ── Account / devices ── */}
      <div className="panel card">
        <h3>Account</h3>
        {config.paired && config.account ? (
          <div className="row between">
            <div>
              <div><b>{config.account.handle}</b> <span className="chip chip-brass">{config.account.role}</span> {config.account.tier && <span className="chip">{config.account.tier}</span>}</div>
              <div className="faint" style={{ fontSize: "0.8rem", marginTop: 4 }}>Channels: {config.account.channels.join(", ")}</div>
            </div>
            <button className="btn-ghost btn-sm" onClick={() => api.unpair().then(ctx.reload).catch(() => ctx.reload())}>Unpair device</button>
          </div>
        ) : (
          <div className="stack">
            <p className="muted" style={{ margin: 0 }}>Pair with a code from the website (Account → Devices → Add device).</p>
            <label className="field"><span>Pairing code</span>
              <input value={pairCode} onChange={(e) => setPairCode(e.target.value)}
                autoComplete="off" autoCorrect="off" autoCapitalize="characters" spellCheck={false}
                style={{ textTransform: "uppercase" }} placeholder="GEC-K7M2Q9" /></label>
            {err && <div className="err">{err}</div>}
            <div><button className="btn-spark btn-sm" disabled={!pairCode.trim()} onClick={pair}>Pair device</button></div>
          </div>
        )}
      </div>

      {/* ── Startup ── */}
      <div className="panel card">
        <h3>Startup</h3>
        <label className="check">
          <input type="checkbox" checked={launchAtLogin} onChange={(e) => toggleLaunchAtLogin(e.target.checked)} />
          Launch GEC Uplink at login
        </label>
        <p className="faint" style={{ margin: "0.2rem 0 0.7rem", fontSize: "0.78rem" }}>Starts Uplink automatically when you log in to your computer.</p>
        <label className="check">
          <input type="checkbox" checked={config.start_in_tray} onChange={(e) => api.setStartInTray(e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
          Start in the menu bar (don't open the window)
        </label>
        <p className="faint" style={{ margin: "0.2rem 0 0", fontSize: "0.78rem" }}>Launches straight to the menu-bar icon — click it any time to open the window.</p>
      </div>

      {/* ── Per-install settings ── */}
      <div className="panel card">
        <div className="row between"><h3 style={{ margin: 0 }}>WoW installs</h3>
          <div className="row" style={{ gap: "0.5rem" }}>
            {/* Re-run the setup wizard's Find-WoW step to pick up a newly-installed WoW
                version. The wizard is non-destructive — it only ADDS installs and
                reconciles on-disk addon versions; it never removes an install or clears
                its addon selections, channels, pins, or sync state. Setup's initial step
                is `paired ? 1 : 0`, so a paired user re-enters straight at Find-WoW. */}
            <button className="btn-ghost btn-sm" onClick={() => ctx.openSetup()}>Added a new WoW version? Find installs</button>
            <button className="btn-brass btn-sm" onClick={browseAdd}>＋ Add install</button>
          </div>
        </div>
        <div className="stack" style={{ marginTop: "0.7rem" }}>
          {config.installs.length === 0 && <p className="muted">No installs yet.</p>}
          {config.installs.map((i) => (
            <div key={i.id} className="panel" style={{ padding: "0.8rem 1rem" }}>
              <div className="row between">
                <div>
                  <b>{i.label}</b> <span className="chip">{i.flavor}</span> {!i.online && <span className="badge-offline">⚠ unplugged</span>}
                  <div className="faint tabular" style={{ fontSize: "0.72rem" }}>{i.path}</div>
                </div>
                <div className="row">
                  <button className="btn-ghost btn-sm" onClick={() => api.resetAddons(i.id).then(ctx.reload).catch(() => ctx.reload())}>Reset addons</button>
                  <button className="btn-ghost btn-sm" onClick={() => api.removeInstall(i.id).then(ctx.reload).catch(() => ctx.reload())}>Remove</button>
                </div>
              </div>
              <div className="grid2" style={{ marginTop: "0.6rem" }}>
                <label className="field"><span>Default channel</span>
                  <Select value={i.channel}
                    options={entitled.map((c) => ({ value: c.slug, label: `${c.name}${c.badge ? ` ${c.badge}` : ""}` }))}
                    onChange={(v) => api.updateInstall(i.id, i.label, v, i.auto_update)
                      .then(() => api.reconcileInstall(i.id))
                      .then(ctx.reload)
                      // Don't swallow a failed channel switch/reconcile — surface it
                      // and re-sync the UI to disk so config and screen can't diverge.
                      .catch((e) => { setErr(String(e)); ctx.reload(); })} />
                </label>
                <label className="check" style={{ alignSelf: "end" }}>
                  <input type="checkbox" checked={i.auto_update} onChange={(e) => api.setInstallAutoUpdateAll(i.id, e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
                  Auto-update all addons here
                </label>
              </div>
              <p className="faint" style={{ margin: "0.4rem 0 0", fontSize: "0.78rem" }}>The default channel &amp; auto-update apply to newly-added addons; the checkbox also sets every addon here now. Tune each addon on the Addons page, or <b>Reset addons</b> to make them all follow these defaults.</p>
            </div>
          ))}
        </div>
      </div>

      {/* ── Server ── (base URL only; sync toggles moved to the Data page, redesign §7) */}
      <div className="panel card">
        <h3>Server</h3>
        <label className="field"><span>Website address <span className="faint">(advanced — leave as is)</span></span>
          <div className="row"><input value={baseUrl} onChange={(e) => setBaseUrl(e.target.value)} placeholder="https://goblineng.co" />
            <button className="btn-brass btn-sm" onClick={saveBaseUrl}>Save</button></div>
        </label>
        {savedNote && <div className="ok" style={{ marginTop: 6 }}>{savedNote}</div>}
        <p className="faint" style={{ margin: "0.4rem 0 0", fontSize: "0.78rem" }}>Where Uplink talks to your account. Sync options live on the <b>Data</b> page.</p>
      </div>

      {/* ── Self-update ── */}
      <UpdatePanel config={config} channels={entitled} reload={ctx.reload} />

    </>
  );
}
