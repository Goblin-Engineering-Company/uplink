import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import gecLogo from "./assets/gec-logo.png";
import { api } from "./lib/api";
import { applyTheme } from "./lib/themes";
import type { AppConfig } from "./lib/types";
import { Setup } from "./views/Setup";
import { TrayPanel } from "./views/TrayPanel";
import { Home } from "./views/Home";
import { Addons } from "./views/Addons";
import { Data } from "./views/Data";
import { Settings } from "./views/Settings";

export type Section = "home" | "addons" | "data" | "settings";

export type Ctx = {
  config: AppConfig;
  reload: () => Promise<void>;
  go: (s: Section) => void;
  // Bumped whenever the server's catalog dirty-stamp moves (uplink:catalog-changed),
  // so catalog-fetching effects can depend on it and re-pull the catalog.
  catalogNonce: number;
  // Re-open / close the setup wizard on demand (Settings "Find installs" re-entry).
  // On genuine first run the wizard shows because `!onboarded`; these drive it AFTER
  // onboarding, so re-running it returns to the app on finish/skip (§2.1–2.2).
  openSetup: () => void;
  closeSetup: () => void;
};

// Backstop heartbeat interval. Window focus covers snappy freshness; this gentle
// interval just keeps state current (the endpoint bumps device last_seen each call,
// so we don't hammer it). Spec §4.1 suggests 5–15 min.
const HEARTBEAT_MS = 10 * 60_000;

export default function App({ surface }: { surface: "tray" | "window" }) {
  const [config, setConfig] = useState<AppConfig | null>(null);
  const [section, setSection] = useState<Section>("home");
  const [err, setErr] = useState<string | null>(null);
  const [version, setVersion] = useState("");
  const [catalogNonce, setCatalogNonce] = useState(0);
  // Re-entered setup (Settings → "Find installs"). First run shows Setup via
  // !onboarded; this lets an already-onboarded user re-open the wizard.
  const [forceSetup, setForceSetup] = useState(false);

  const reload = useCallback(async () => {
    const c = await api.getConfig();
    applyTheme(c.theme);
    setConfig(c);
  }, []);

  useEffect(() => {
    reload().catch((e) => setErr(String(e)));
    getVersion().then(setVersion).catch(() => {}); // real app version, not hardcoded
  }, [reload]);

  // Server-state heartbeat: on launch + window focus + every HEARTBEAT_MS, pull
  // /api/me and reload config. fetch_home refreshes the paired account (role /
  // channels / tier) into config, so a role change or newly-granted channel appears
  // app-wide (e.g. Settings' channel list) WITHOUT re-pairing — not only while the
  // Home view is open. It also compares stamps.catalog and (on a change) emits
  // uplink:catalog-changed, which the listener below turns into a catalogNonce bump.
  // Best-effort.
  useEffect(() => {
    let alive = true;
    const beat = async () => {
      try {
        await api.fetchHome();
        if (alive) await reload();
      } catch {
        /* offline / unpaired — ignore, next tick retries */
      }
    };
    beat();
    const id = window.setInterval(beat, HEARTBEAT_MS);
    window.addEventListener("focus", beat);
    return () => {
      alive = false;
      window.clearInterval(id);
      window.removeEventListener("focus", beat);
    };
  }, [reload]);

  // Catalog dirty-stamp: fetch_home emits uplink:catalog-changed only when the
  // server's stamps.catalog fingerprint actually moves. Bump the nonce so the
  // catalog-fetching views (Addons / Settings) re-pull the catalog.
  useEffect(() => {
    const un = listen("uplink:catalog-changed", () => setCatalogNonce((n) => n + 1));
    return () => { un.then((f) => f()); };
  }, []);

  if (err) return (
    <div className="center-screen">
      <div className="err">Failed to load: {err}</div>
      {/* A transient get_config failure must not brick the window (with start-in-tray
          there may be no window to quit from) — always offer a retry. */}
      <button className="btn-spark btn-sm" style={{ marginTop: "0.8rem" }}
        onClick={() => { setErr(null); reload().catch((e) => setErr(String(e))); }}>
        Retry
      </button>
    </div>
  );
  if (!config) return <div className="center-screen"><span className="spin" /></div>;

  const ctx: Ctx = {
    config, reload, go: setSection, catalogNonce,
    openSetup: () => setForceSetup(true),
    closeSetup: () => setForceSetup(false),
  };

  // The tray dropdown is the compact daily view.
  if (surface === "tray") return <TrayPanel ctx={ctx} />;

  // First run: onboarding flow until the user finishes. Also re-openable after
  // onboarding via forceSetup (Settings "Find installs"); the wizard calls
  // closeSetup on finish/skip to return here.
  if (!config.onboarded || forceSetup) return <Setup ctx={ctx} />;

  return (
    <div className="window">
      <div className="hairline" />
      <div className="shell">
      <nav className="sidebar">
        <div className="brandmark">
          <img src={gecLogo} className="brandmark-logo" alt="GEC" />
          <div>
            <div className="display brass-text" style={{ fontSize: "1.1rem", lineHeight: 1 }}>Uplink</div>
            <div className="kicker" style={{ letterSpacing: "0.2em" }}>Goblin Engineering</div>
          </div>
        </div>
        {NAV.map((n) => (
          <div key={n.id} className={`nav-item ${section === n.id ? "active" : ""}`} onClick={() => setSection(n.id)}>
            <span className="nav-icon">{n.icon}</span>
            {n.label}
          </div>
        ))}
        <div className="sidebar-foot">
          <div className="row" style={{ gap: "0.4rem" }}>
            <span className={`dot ${config.paired ? "dot-on" : "dot-off"}`} />
            {config.account ? config.account.handle : "not paired"}
          </div>
          <div style={{ marginTop: 4 }}>{version ? `v${version}` : ""}</div>
        </div>
      </nav>
      <main className="content">
        {section === "home" && <Home ctx={ctx} />}
        {section === "addons" && <Addons ctx={ctx} />}
        {section === "data" && <Data ctx={ctx} />}
        {section === "settings" && <Settings ctx={ctx} />}
      </main>
      </div>
    </div>
  );
}

const NAV: Array<{ id: Section; label: string; icon: string }> = [
  { id: "home", label: "Home", icon: "⌂" },
  { id: "addons", label: "Addons", icon: "⬡" },
  { id: "data", label: "Data", icon: "≋" },
  { id: "settings", label: "Settings", icon: "⚙" },
];
