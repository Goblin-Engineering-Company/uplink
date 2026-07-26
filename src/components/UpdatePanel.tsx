// Self-update panel (Settings → "Uplink updates"). Talks to the runtime-gated
// Rust commands (channel + device token per request). Copy stays plain-language.
import { useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import { api } from "../lib/api";
import { Select } from "./Select";
import type { AppConfig, ChannelDef } from "../lib/types";

type Phase =
  | { k: "idle" }
  | { k: "checking" }
  | { k: "uptodate" }
  | { k: "available"; version: string }
  | { k: "revoked"; was: string }
  | { k: "installing"; pct: number | null }
  | { k: "error" };

export function UpdatePanel({
  config,
  channels,
  reload,
}: {
  config: AppConfig;
  channels: ChannelDef[];
  reload: () => Promise<void> | void;
}) {
  const [version, setVersion] = useState<string>("");
  const [phase, setPhase] = useState<Phase>({ k: "idle" });
  const mounted = useRef(true);

  useEffect(() => {
    getVersion().then(setVersion).catch(() => setVersion(""));
  }, []);

  useEffect(() => {
    mounted.current = true;
    const un = listen<number | null>("uplink:update-progress", (e) => {
      if (mounted.current) setPhase({ k: "installing", pct: e.payload });
    });
    return () => { mounted.current = false; un.then((f) => f()); };
  }, []);

  async function checkNow() {
    setPhase({ k: "checking" });
    try {
      const s = await api.checkSelfUpdate();
      if (s.kind === "available") setPhase({ k: "available", version: s.version });
      else if (s.kind === "channel_revoked") {
        setPhase({ k: "revoked", was: s.was });
        await reload(); // channel was reset to public server-side
      } else setPhase({ k: "uptodate" });
    } catch {
      setPhase({ k: "error" });
    }
  }

  async function install() {
    setPhase({ k: "installing", pct: null });
    try {
      await api.installSelfUpdate(); // relaunches on success; may not return
    } catch {
      setPhase({ k: "error" });
    }
  }

  // Flatpak (Steam Deck) builds run in a read-only sandbox and can't self-update —
  // hide the check/install controls and the channel/auto settings; point at Flatpak.
  if (config.is_flatpak) {
    return (
      <div className="panel card">
        <h3>Uplink updates</h3>
        <p className="muted" style={{ margin: 0 }}>
          You're using Uplink <span className="tabular">{version || "…"}</span>.
        </p>
        <p className="faint" style={{ margin: "0.5rem 0 0", fontSize: "0.85rem" }}>
          Updates on the Steam Deck are managed by Flatpak — run <code>flatpak update</code>, or enable automatic updates in Discover.
        </p>
      </div>
    );
  }

  return (
    <div className="panel card">
      <h3>Uplink updates</h3>
      <div className="row between" style={{ alignItems: "center" }}>
        <p className="muted" style={{ margin: 0 }}>
          You're using Uplink <span className="tabular">{version || "…"}</span>.
        </p>
        {phase.k !== "checking" && phase.k !== "installing" && (
          <button className="btn-brass btn-sm" onClick={checkNow}>Check for updates</button>
        )}
      </div>

      <div className="grid2" style={{ marginTop: "0.7rem" }}>
        <label className="field"><span>Update channel</span>
          <Select
            value={config.self_update_channel}
            options={channels.map((c) => ({ value: c.slug, label: `${c.name}${c.badge ? ` ${c.badge}` : ""}` }))}
            onChange={(v) => api.setSelfUpdateChannel(v).then(reload).catch(() => reload())}
          />
        </label>
        <label className="field"><span>Auto-update daily at</span>
          <input
            type="time"
            defaultValue={config.auto_update_time}
            onBlur={(e) => api.setAutoUpdateTime(e.target.value || "03:00").then(reload).catch(() => reload())}
          />
        </label>
      </div>
      <label className="check" style={{ marginTop: "0.5rem" }}>
        <input
          type="checkbox"
          checked={config.auto_update_enabled}
          onChange={(e) => api.setAutoUpdate(e.target.checked).then(reload).catch(() => reload())}
        />
        Automatically update Uplink and flagged installs on the schedule above
      </label>

      {phase.k === "checking" && <p className="faint" style={{ margin: "0.5rem 0 0" }}>Checking…</p>}
      {phase.k === "uptodate" && <div className="ok" style={{ marginTop: "0.5rem" }}>You're up to date.</div>}
      {phase.k === "revoked" && (
        <div className="err" style={{ marginTop: "0.5rem" }}>
          Access to the {phase.was} channel was removed — switched to Public.
        </div>
      )}
      {phase.k === "available" && (
        <div className="stack" style={{ marginTop: "0.6rem" }}>
          <p style={{ margin: 0 }}>A newer version (<b className="tabular">{phase.version}</b>) is available.</p>
          <div><button className="btn-spark btn-sm" onClick={install}>Update now</button></div>
        </div>
      )}
      {phase.k === "installing" && (
        <p className="faint" style={{ margin: "0.5rem 0 0" }}>
          {phase.pct === null ? "Downloading the update…" : `Downloading the update… ${phase.pct}%`}
        </p>
      )}
      {phase.k === "error" && (
        <div className="err" style={{ marginTop: "0.5rem" }}>Couldn't check for updates right now. Please try again later.</div>
      )}
    </div>
  );
}
