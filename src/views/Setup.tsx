import { useEffect, useState } from "react";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { CatalogAddon, DetectedInstall, Install } from "../lib/types";
import { supportsFlavor } from "../lib/types";
import { AddonDot } from "../components/AddonDot";
import { PairSession } from "../components/PairSession";

// First-run: Pair → Find WoW → Pick addons. Collapses to the tray when done.
export function Setup({ ctx }: { ctx: Ctx }) {
  const [step, setStep] = useState(ctx.config.paired ? 1 : 0);

  return (
    <div className="center-screen">
      <div className="setup">
        <div className="step-dots">
          {[0, 1, 2].map((i) => <i key={i} className={i <= step ? "on" : ""} />)}
        </div>
        {step === 0 && <StepPair ctx={ctx} next={() => setStep(1)} />}
        {step === 1 && <StepFindWow ctx={ctx} next={() => setStep(2)} />}
        {step === 2 && <StepPickAddons ctx={ctx} />}
      </div>
    </div>
  );
}

function StepPair({ ctx, next }: { ctx: Ctx; next: () => void }) {
  return (
    <div className="panel card">
      <div className="kicker">Step 1 of 3</div>
      <h1 className="display" style={{ margin: "0.2rem 0 0.4rem" }}>Pair this device</h1>
      <p className="muted">
        Pairing links this device to your account so Uplink can <b>sync your addon data to the website</b> and
        let you into any member-only addon channels. It's <b>optional</b>: skip now and pair later from <b>Settings</b>.
      </p>
      <div style={{ marginTop: "0.9rem" }}>
        <PairSession onPaired={async () => { await ctx.reload(); next(); }} />
      </div>
      <div className="row between" style={{ marginTop: "1rem" }}>
        <button className="btn-ghost" onClick={next}>Skip for now</button>
      </div>
    </div>
  );
}

function StepFindWow({ ctx, next }: { ctx: Ctx; next: () => void }) {
  const [detected, setDetected] = useState<DetectedInstall[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    api.detectInstalls().then(setDetected).catch((e) => setErr(String(e)));
  }, []);

  async function add(d: DetectedInstall) {
    setBusy(true); setErr(null);
    try { await api.addInstall(d.path, d.label); await ctx.reload();
      setDetected((prev) => prev?.map((x) => x.path === d.path ? { ...x, already_added: true } : x) ?? null);
    } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  }

  async function browse() {
    const path = await api.browseFolder();
    if (!path || typeof path !== "string") return;
    setBusy(true); setErr(null);
    try {
      // Enumerate every WoW flavor under the browsed folder (Retail/Classic/PTR/…)
      // and MERGE them into the detected list as addable rows — don't auto-add. The
      // user clicks Add on the ones they want, same as the auto-detected rows.
      const found = await api.enumerateInstallsAt(path);
      setDetected((prev) => {
        const byPath = new Map((prev ?? []).map((x) => [x.path, x]));
        for (const d of found) byPath.set(d.path, d); // dedupe by path; browsed wins
        return [...byPath.values()];
      });
    } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  }

  // Skip: finish onboarding without adding an install (fixes the Continue-disabled
  // dead-end). Does NOT advance into an empty step 3; closeSetup returns a re-entered
  // wizard to the app (harmless on genuine first run — onboarded flips true anyway).
  async function skip() {
    setBusy(true); setErr(null);
    try {
      await api.markOnboarded();
      await ctx.reload();
      ctx.closeSetup();
    } catch (e) { setErr(String(e)); setBusy(false); }
  }

  return (
    <div className="panel card">
      <div className="kicker">Step 2 of 3</div>
      <h1 className="display" style={{ margin: "0.2rem 0 0.4rem" }}>Point to your World of Warcraft folder</h1>
      <p className="muted">Uplink finds every version under your World of Warcraft folder: Retail, Classic, PTR. If your game lives on an external drive or a custom path, browse to that folder and we'll list what's inside.</p>
      {detected === null ? <div className="row"><span className="spin" /> scanning…</div> : (
        <ul className="list-reset" style={{ marginTop: "0.6rem" }}>
          {detected.length === 0 && <li className="muted">Nothing detected automatically. Browse to your WoW folder.</li>}
          {detected.map((d) => (
            <li key={d.path} className="row between panel" style={{ padding: "0.6rem 0.8rem", marginBottom: "0.4rem" }}>
              <div>
                <div>{d.label} <span className="chip">{d.flavor}{d.flavor === "Classic" ? " · untested" : ""}</span></div>
                <div className="faint tabular" style={{ fontSize: "0.72rem" }}>{d.path}</div>
              </div>
              {d.already_added ? <span className="ok">added</span> :
                <button className="btn-brass btn-sm" disabled={busy} onClick={() => add(d)}>Add</button>}
            </li>
          ))}
        </ul>
      )}
      {err && <div className="err">{err}</div>}
      <div className="row between" style={{ marginTop: "0.8rem" }}>
        <button className="btn-ghost" disabled={busy} onClick={browse}>＋ Browse to your WoW folder…</button>
        <div className="row" style={{ gap: "0.5rem" }}>
          <button className="btn-ghost" disabled={busy} onClick={skip}>Skip for now</button>
          <button className="btn-spark" disabled={ctx.config.installs.length === 0} onClick={next}>Continue</button>
        </div>
      </div>
    </div>
  );
}

function StepPickAddons({ ctx }: { ctx: Ctx }) {
  const [addons, setAddons] = useState<CatalogAddon[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const installs = ctx.config.installs;
  const [installId, setInstallId] = useState(installs[0]?.id ?? "");
  // `ctx` is rebuilt every App render, so it can never be a dep; `reload` is the only
  // part this effect uses and it IS stable (a useCallback with an empty dep list).
  const { reload } = ctx;

  useEffect(() => {
    // reconcile against on-disk addons first so anything already installed (by
    // any method) shows as installed, then load the roster.
    api.reconcileInstalled().then(async (c) => { setAddons(c.addons); await reload(); }).catch((e) => setErr(String(e)));
  }, [reload]);

  const install: Install | undefined = installs.find((i) => i.id === installId);
  const selected = new Set(install?.addons.filter((a) => a.enabled).map((a) => a.slug));

  async function toggle(slug: string, on: boolean) {
    await api.setAddonSelected(installId, slug, on);
    await ctx.reload();
  }

  async function finish() {
    setBusy("install");
    try {
      // install every selected addon on every install at its channel's latest
      const cat = addons ?? [];
      for (const inst of ctx.config.installs) {
        for (const a of inst.addons.filter((x) => x.enabled && !x.installed_version)) {
          const def = cat.find((c) => c.slug === a.slug);
          const latest = def?.channels[inst.channel] ?? def?.channels["public"];
          // Never push a Retail-only addon into a Classic install — this loop runs over
          // EVERY install, and the selection is made against one of them. (Rust refuses
          // it too; this just avoids the round-trip and the error.)
          if (def && latest && supportsFlavor(def, inst.flavor)) {
            try { await api.installAddon(inst.id, a.slug, latest.version, latest.url, inst.channel); } catch { /* keep going */ }
          }
        }
      }
      await api.reportInstalls().catch(() => {});
      await api.markOnboarded();
      await ctx.reload();
      ctx.closeSetup(); // return a re-entered wizard to the app (harmless on first run)
      // Final setup step: pull the newest Uplink. A freshly-downloaded installer can already be behind the
      // published build, so check now and auto-install if there's an update — install_self_update downloads,
      // installs, and relaunches into the new version (onboarded is already set, so it won't re-run setup).
      // Best-effort: never block finishing setup on it (offline / no endpoint / Flatpak just falls through).
      setBusy("update");
      try {
        const st = await api.checkSelfUpdate();
        if (st.kind === "available") await api.installSelfUpdate();
      } catch { /* finish anyway */ }
    } finally { setBusy(null); }
  }

  return (
    <div className="panel card">
      <div className="kicker">Step 3 of 3</div>
      <h1 className="display" style={{ margin: "0.2rem 0 0.4rem" }}>Pick your addons</h1>
      {installs.length > 1 && (
        <div className="toolbar">
          {installs.map((i) => (
            <span key={i.id} className={`pill ${i.id === installId ? "active" : ""}`} onClick={() => setInstallId(i.id)}>
              {i.label} · {i.flavor}
            </span>
          ))}
        </div>
      )}
      {addons === null ? <div className="row"><span className="spin" /> loading catalog…</div> : (
        <ul className="list-reset">
          {addons.map((a) => {
            // Shown but not selectable on a flavor it doesn't support, with the reason
            // in place of the status chip — hiding it would look like a missing addon.
            const ok = !install || supportsFlavor(a, install.flavor);
            const only = (a.flavors ?? []).join(" / ");
            return (
              <label key={a.slug} className="check" title={ok ? undefined : `Not available for ${install?.flavor}`}>
                <input type="checkbox" disabled={!ok} checked={ok && selected.has(a.slug)} onChange={(e) => toggle(a.slug, e.target.checked)} />
                <AddonDot slug={a.slug} />
                <span style={{ flex: 1, opacity: ok ? 1 : 0.55 }}>{a.name} <span className="faint">{a.channels[install?.channel ?? "public"]?.version ?? a.channels["public"]?.version ?? "—"}</span></span>
                <span className="chip">{ok ? a.status : only ? `${only} only` : "Not for this flavor"}</span>
              </label>
            );
          })}
        </ul>
      )}
      {err && <div className="err">{err}</div>}
      <p className="faint" style={{ margin: "0.9rem 0 0", fontSize: "0.76rem" }}>
        Uplink syncs the data your installed addons record to your account. You can turn this off anytime in Settings.
      </p>

      <div className="cog-rule" style={{ margin: "0.9rem 0 0.6rem" }} />
      <label className="check">
        <input type="checkbox" checked={ctx.config.start_in_tray}
          onChange={(e) => api.setStartInTray(e.target.checked).then(ctx.reload).catch(() => ctx.reload())} />
        Start Uplink in the menu bar (no window) on launch
      </label>
      <p className="faint" style={{ margin: "0.3rem 0 0", fontSize: "0.76rem" }}>
        Uplink lives in your <b>menu bar</b> (top-right on Mac) or <b>system tray</b> (Windows/Linux), and keeps
        running there after you close this window. Click its icon anytime to open Uplink or sync.
      </p>

      <div className="row between" style={{ marginTop: "0.6rem" }}>
        <button className="btn-ghost" onClick={() => { api.markOnboarded().then(ctx.reload).then(() => ctx.closeSetup()).catch((e) => setErr(String(e))); }}>Skip install</button>
        <button className="btn-spark" disabled={!!busy} onClick={finish}>
          {busy === "update" ? <><span className="spin" /> Updating Uplink…</> : busy ? <><span className="spin" /> Installing…</> : "Install & finish"}
        </button>
      </div>
    </div>
  );
}
