import { useEffect, useRef, useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { CatalogAddon, StreamState, UpdateItem } from "../lib/types";
import { AddonDot } from "../components/AddonDot";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { LogicalSize } from "@tauri-apps/api/dpi";
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
  const bodyRef = useRef<HTMLDivElement>(null);

  // Pull fresh catalog / updates / sync. Runs on open AND every time the panel regains focus, so a purge
  // or update done in the main window (a SEPARATE webview) is reflected here immediately instead of going
  // stale. Also re-reads config (roster) so the addon list tracks what's actually installed.
  async function refresh() {
    await ctx.reload().catch(() => {});
    const [c, u, s] = await Promise.all([
      api.fetchCatalog().then((r) => r.addons).catch(() => null),
      api.checkUpdates().catch(() => null),
      api.getSyncStatus().catch(() => null),
    ]);
    if (c) setCatalog(c);
    if (u) setUpdates(u);
    if (s) setSync(s);
  }

  useEffect(() => {
    refresh();
    const w = getCurrentWebviewWindow();
    const un = w.onFocusChanged(({ payload: focused }) => { if (focused) refresh(); });
    return () => { un.then((f) => f()); };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Grow the tray window to fit its content (up to a cap) so the Update / Sync buttons are never below a
  // scroll. Re-measures whenever the visible content changes. Preserves the current width; clamps height.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const el = bodyRef.current?.closest(".tray") as HTMLElement | null;
      if (!el) return;
      const w = getCurrentWebviewWindow();
      try {
        const factor = await w.scaleFactor();
        const cur = await w.innerSize();
        const widthLogical = Math.round(cur.width / factor);
        const h = Math.min(720, Math.max(160, Math.ceil(el.scrollHeight) + 2));
        if (!cancelled) await w.setSize(new LogicalSize(widthLogical, h));
      } catch { /* window sizing best-effort; content still scrolls if it fails */ }
    })();
    return () => { cancelled = true; };
  });

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

  async function updateAll() {
    setBusy(true);
    try {
      for (const inst of config.installs) await api.updateAll(inst.id).catch(() => []);
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

      <div className="tray-body" ref={bodyRef}>
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

        {updates.length > 0 && (
          <>
            <div className="cog-rule" style={{ margin: "0.6rem 0" }} />
            <div className="kicker" style={{ marginBottom: "0.3rem" }}>Updates</div>
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
            <button className="btn-spark btn-sm" style={{ width: "100%", marginTop: "0.5rem" }} disabled={busy} onClick={updateAll}>
              {busy ? <span className="spin" /> : `Update all (${updates.length})`}
            </button>
          </>
        )}
      </div>

      <div className="tray-foot">
        <button className="btn-brass btn-sm" style={{ flex: 1 }} onClick={openMain}>Open Uplink</button>
      </div>
    </div>
  );
}
