import { useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import { api } from "../lib/api";
import type { PairSessionInfo } from "../lib/types";

// QR pairing (spec 2026-07-26) — the ONE pairing UI, shared by Setup step 1
// and Settings → Account (re-pair after an unpair). Starts an app-initiated
// session and offers the three faces of the same flow:
//   1. a QR code (scan with a phone that's signed in to the site),
//   2. an "Open approval page" button (same-machine browser, zero typing),
//   3. the short code, for typing at goblineng.co/pair on any signed-in browser.
// Then polls until a signed-in browser approves, and hands the account back.
export function PairSession({ onPaired }: { onPaired: () => void | Promise<void> }) {
  const [ses, setSes] = useState<PairSessionInfo | null>(null);
  const [phase, setPhase] = useState<"starting" | "waiting" | "approved" | "denied" | "expired" | "error">("starting");
  const [err, setErr] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [nonce, setNonce] = useState(0); // bump = start a fresh session

  // Start (or restart) a session. The ref guards React 18 dev double-mount
  // from burning two sessions against the server's rate limit.
  const startedFor = useRef(-1);
  useEffect(() => {
    if (startedFor.current === nonce) return;
    startedFor.current = nonce;
    let gone = false;
    setPhase("starting"); setErr(null); setSes(null);
    api.pairStart()
      .then((s) => { if (!gone) { setSes(s); setPhase("waiting"); } })
      .catch((e) => { if (!gone) { setErr(String(e)); setPhase("error"); } });
    return () => { gone = true; };
  }, [nonce]);

  // Draw the QR once the session (and its canvas) exist. Dark modules on the
  // parchment light keep phone cameras happy on our dark UI.
  useEffect(() => {
    if (!ses || !canvasRef.current) return;
    QRCode.toCanvas(canvasRef.current, ses.approve_url, {
      width: 168,
      margin: 1,
      errorCorrectionLevel: "M",
      color: { dark: "#221a10", light: "#f2e8d5" },
    }).catch(() => { /* canvas gone mid-render */ });
  }, [ses]);

  // The poll loop: every poll_interval until a terminal state. Network blips
  // are swallowed — the next tick retries; the session's 15-minute expiry is
  // the real deadline and comes back from the server as status "expired".
  useEffect(() => {
    if (!ses || phase !== "waiting") return;
    let busy = false;
    const t = setInterval(async () => {
      if (busy) return;
      busy = true;
      try {
        const r = await api.pairPoll(ses.code, ses.secret);
        if (r.status === "approved") { clearInterval(t); setPhase("approved"); await onPaired(); }
        else if (r.status === "denied") { clearInterval(t); setPhase("denied"); }
        else if (r.status === "expired") { clearInterval(t); setPhase("expired"); }
      } catch { /* transient — retry next tick */ }
      busy = false;
    }, Math.max(1.5, ses.poll_interval) * 1000);
    return () => clearInterval(t);
  }, [ses, phase]); // eslint-disable-line react-hooks/exhaustive-deps

  if (phase === "starting") {
    return <div className="row" style={{ gap: "0.5rem" }}><span className="spin" /> getting a pairing code…</div>;
  }
  if (phase === "error") {
    return (
      <div className="stack">
        {err && <div className="err">{err}</div>}
        <div><button className="btn-brass btn-sm" onClick={() => setNonce((n) => n + 1)}>Try again</button></div>
      </div>
    );
  }
  if (phase === "denied" || phase === "expired") {
    return (
      <div className="stack">
        <p className="muted" style={{ margin: 0 }}>
          {phase === "denied" ? "That pairing was denied on the website." : "The pairing code expired (they last 15 minutes)."}
        </p>
        <div><button className="btn-brass btn-sm" onClick={() => setNonce((n) => n + 1)}>Get a fresh code</button></div>
      </div>
    );
  }
  if (phase === "approved" || !ses) {
    return <div className="row" style={{ gap: "0.5rem" }}><span className="spin" /> paired, finishing up…</div>;
  }

  return (
    <div className="row" style={{ gap: "1rem", alignItems: "flex-start", flexWrap: "wrap" }}>
      <div style={{ background: "#f2e8d5", borderRadius: 10, padding: 6, lineHeight: 0, flex: "0 0 auto" }}>
        <canvas ref={canvasRef} />
      </div>
      <div className="stack" style={{ flex: "1 1 14rem", minWidth: 0 }}>
        <p className="muted" style={{ margin: 0 }}>
          <b>Scan with your phone</b> and approve. Or approve right here:
        </p>
        <div>
          <button className="btn-brass btn-sm" onClick={() => api.openExternal(ses.approve_url)}>
            Open approval page ↗
          </button>
        </div>
        <p className="faint" style={{ fontSize: "0.74rem", margin: 0 }}>
          Or enter this code at <b>goblineng.co/pair</b> in any signed-in browser:
        </p>
        <div className="tabular" style={{ fontSize: "1.15rem", letterSpacing: "0.12em", fontWeight: 600 }}>
          {ses.code}
        </div>
        <p className="faint row" style={{ fontSize: "0.74rem", margin: 0, gap: "0.4rem" }}>
          <span className="spin" /> waiting for approval…
        </p>
      </div>
    </div>
  );
}
