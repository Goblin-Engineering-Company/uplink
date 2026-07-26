import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { Me } from "../lib/types";

// ── tiny formatters for the per-addon cards (server sends copper / seconds) ──
const gold = (copper: number) => `${Math.floor(copper / 10000).toLocaleString()}g`;
// hours + minutes ("4h 32m"; under an hour just "12m")
const hrs = (sec: number) => {
  const h = Math.floor(sec / 3600);
  const m = Math.round((sec % 3600) / 60);
  return h > 0 ? `${h}h ${m}m` : `${Math.max(1, m)}m`;
};
const pct = (r: number | null) => (r == null ? "—" : `${Math.round(r * 100)}%`);
// One votable row: title link + community votes, with "your vote/boosts live
// HERE" indicators when this user's weight sits on the row.
function VoteRow({ v }: { v: { number: number; title: string; url: string; votes: number; my_votes?: number; my_boosts?: number } }) {
  const mine = (v.my_votes ?? 0) > 0;
  const boosts = v.my_boosts ?? 0;
  return (
    <div className="row between">
      <a onClick={() => api.openExternal(v.url)} style={{ cursor: "pointer" }}>
        #{v.number} {v.title} ↗
      </a>
      <span className="row" style={{ gap: "0.35rem", flexShrink: 0 }}>
        {mine && (
          <span className="chip chip-green" title={boosts > 0 ? `Your vote is here, including ${boosts} boost${boosts === 1 ? "" : "s"}` : "Your free vote is here"}>
            ✓ yours{boosts > 0 ? ` ⚡${boosts}` : ""}
          </span>
        )}
        <span className="chip chip-spark">{v.votes} votes</span>
      </span>
    </div>
  );
}

const ago = (iso: string | null) => {
  if (!iso) return null;
  const s = Math.max(0, (Date.now() - Date.parse(iso.replace(" ", "T") + "Z")) / 1000);
  if (s < 90) return "synced just now";
  if (s < 5400) return `synced ${Math.round(s / 60)}m ago`;
  if (s < 129600) return `synced ${Math.round(s / 3600)}h ago`;
  return `synced ${Math.round(s / 86400)}d ago`;
};

// Engagement hub, fed by one GET /api/me (spec §9.5). Stubs gracefully when the
// device isn't paired (the call 401s).
export function Home({ ctx }: { ctx: Ctx }) {
  const [me, setMe] = useState<Me | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const dismissed = ctx.config.dismissed_broadcast;
  const siteBase = (ctx.config.base_url || "https://goblineng.co").replace(/\/+$/, "");

  useEffect(() => {
    let alive = true;
    // Broadcasts are pull-based (GET /api/me): a server-side delete or edit propagates
    // here on the next fetch. Refetch on focus/visibility + a light poll so a retracted
    // broadcast clears from the device promptly (it could point at something no longer
    // available), not just on app restart. A null broadcast auto-hides below.
    const load = () =>
      api.fetchHome()
        .then((m) => { if (alive) { setMe(m); setErr(null); } })
        .catch((e) => { if (alive) setErr(String(e)); });
    load();
    const onFocus = () => load();
    const onVisible = () => { if (!document.hidden) load(); };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onVisible);
    const poll = window.setInterval(load, 60_000);
    return () => {
      alive = false;
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onVisible);
      window.clearInterval(poll);
    };
  }, []);

  async function dismiss(id: number) {
    await api.dismissBroadcast(id);
    await ctx.reload();
  }

  return (
    <>
      <div className="page-head">
        <div className="kicker">Welcome{me ? `, ${me.handle}` : ""}</div>
        <h1>Home</h1>
      </div>

      {!ctx.config.paired && (
        <div className="banner">Pair this device in <b>Settings</b> to see your broadcast, votes, and contribution stats.</div>
      )}
      {err && ctx.config.paired && <div className="banner"><span className="err">Couldn’t load your feed:</span> {err}</div>}

      {/* Upload freeze (spec §7b): the server is refusing ingest globally — tell the
          user WHY (verbatim message) so a stalled sync isn't a mystery. */}
      {me?.service?.uploads === "frozen" && (
        <div className="banner err">
          <b>⏸ Uploads paused.</b> {me.service.message || "The server is temporarily not accepting uploads. Your data is safe and will sync once it resumes."}
        </div>
      )}

      {/* Persistent data-quality nags mirrored from ingest (§10.5). */}
      {(me?.data_warnings ?? []).length > 0 && (
        <div className="banner">
          {me!.data_warnings!.map((w, i) => (
            <div key={i}>
              {/* The server message is self-contained (already names the addon), so we DON'T prepend
                  w.addon — that produced the doubled "Haul: Haul:". Only fall back to the addon label
                  if a message ever arrives without it. */}
              <b>⚠ {w.addon && !w.message.toLowerCase().startsWith(w.addon.toLowerCase()) ? `${w.addon}: ` : ""}</b>{w.message}{w.count ? ` (${w.count})` : ""}
            </div>
          ))}
        </div>
      )}

      {me?.broadcast && me.broadcast.id !== dismissed && (
        <div className="panel-brass rivets rise rise-1" style={{ marginBottom: "1rem" }}>
          <i className="rivets-b" />
          <div className="panel-brass-in" style={{ padding: "1rem 1.1rem" }}>
            <div className="row between">
              <div className="kicker">📢 Broadcast</div>
              <button className="btn-ghost btn-sm" onClick={() => dismiss(me.broadcast!.id)}>Dismiss</button>
            </div>
            <h3 style={{ margin: "0.4rem 0 0.3rem" }}>{me.broadcast.title}</h3>
            <div className="muted broadcast-body">
              <ReactMarkdown
                components={{
                  // Route through the OS browser (opener plugin) like every other
                  // link in the app — a plain target="_blank" can't navigate under
                  // the webview's default-src 'self' CSP and leaves no way back.
                  a: ({ href, children }) => (
                    <a onClick={() => href && api.openExternal(href)} style={{ cursor: "pointer" }}>{children}</a>
                  ),
                }}
              >
                {me.broadcast.body}
              </ReactMarkdown>
            </div>
          </div>
        </div>
      )}

      {/* Full-width stack: votes → profile → per-addon cards → what's new. */}
      <div className="stack" style={{ gap: "1rem" }}>
        <div className="panel card lift rise rise-2">
          <div className="row between" style={{ flexWrap: "wrap", gap: "0.4rem" }}>
            <h3 style={{ margin: 0 }}>🗳 Help shape what’s next</h3>
            {/* Spend-encouragement chips: the free vote + boost-credit balance. */}
            {me?.vote_state && (
              <span className="row" style={{ gap: "0.4rem" }}>
                <span className={`chip ${me.vote_state.free_vote_available ? "chip-green" : ""}`}
                  title={me.vote_state.free_vote_available ? "You have a free vote this cycle — spend it!" : "Free vote cast this cycle"}>
                  {me.vote_state.free_vote_available ? "1 free vote available" : "free vote cast ✓"}
                </span>
                <span className="chip chip-spark" title="Boost credits — spend them as extra votes; refunded on anything that doesn't ship">
                  ⚡ {me.vote_state.boost_credits} boost{me.vote_state.boost_credits === 1 ? "" : "s"}
                </span>
              </span>
            )}
          </div>
          {(me?.votables ?? []).length === 0 && <p className="muted">No open votes right now.</p>}
          <div className="stack" style={{ marginTop: "0.5rem" }}>
            {me?.votables.slice(0, 5).map((v) => <VoteRow key={v.number} v={v} />)}
          </div>
          {/* The server sends the FULL votable list in popularity order; the top 5
              render inline and the rest sit behind a twirl-down. */}
          {me && me.votables.length > 5 && (
            <details style={{ marginTop: "0.5rem" }}>
              <summary className="faint" style={{ cursor: "pointer", fontSize: "0.8rem" }}>
                +{me.votables.length - 5} more
              </summary>
              <div className="stack" style={{ marginTop: "0.5rem" }}>
                {me.votables.slice(5).map((v) => <VoteRow key={v.number} v={v} />)}
              </div>
            </details>
          )}
          {me && (me.votables?.length ?? 0) > 0 && (
            <a onClick={() => api.openExternal(`${siteBase}/roadmap`)}
              className="faint" style={{ cursor: "pointer", display: "inline-block", marginTop: "0.6rem", fontSize: "0.8rem" }}>
              See all &amp; vote ↗
            </a>
          )}
        </div>

        {/* Clicking the card opens the user's full profile ("My Data") on the site —
            characters, sessions, catches, fishing stats. Only active once paired. */}
        <div className="panel card lift rise rise-3"
          role={me ? "button" : undefined}
          onClick={() => { if (me) api.openExternal(`${siteBase}/me`); }}
          style={me ? { cursor: "pointer" } : undefined}
          title={me ? "Open your full profile on goblineng.co" : undefined}>
          <div className="row between">
            <h3 style={{ margin: 0 }}>Profile</h3>
            {me && <span className="faint" style={{ fontSize: "0.8rem" }}>View profile ↗</span>}
          </div>
          <div className="grid2" style={{ marginTop: "0.5rem" }}>
            <div className="stat">
              <div className="n tabular">{me?.rank.xp ?? "—"}</div>
              <div className="l">{me?.rank.name ?? "Rank"} · XP</div>
            </div>
            <div className="stat">
              <div className="n tabular">{me?.discoveries_week ?? "—"}</div>
              <div className="l">Discoveries / week</div>
            </div>
          </div>
        </div>
      </div>

      {/* Per-addon gameplay cards (profile-panel design 2026-07-25): a card only
          renders when the server sent data for that addon; with one addon the
          single card takes the full row. Click-through to the matching My Data
          page, same pattern as the contribution card. */}
      {(me?.addons?.sbf || me?.addons?.haul) && (
        <div className={me.addons.sbf && me.addons.haul ? "grid2" : undefined} style={{ marginTop: "1rem" }}>
          {me.addons.sbf && (() => { const s = me.addons.sbf; return (
            <div className="panel card lift" role="button" style={{ cursor: "pointer" }}
              onClick={() => api.openExternal(`${siteBase}/me/fishing`)}
              title="Open your fishing stats on goblineng.co">
              <div className="row between">
                <h3 style={{ margin: 0 }}>🎣 Single Button Fishing</h3>
                <span className="faint" style={{ fontSize: "0.8rem" }}>Fishing stats ↗</span>
              </div>
              <div className="grid2" style={{ marginTop: "0.5rem" }}>
                <div className="stat"><div className="n tabular">{s.fish.toLocaleString()}</div><div className="l">Fish caught</div></div>
                <div className="stat"><div className="n tabular">{pct(s.catch_rate)}</div><div className="l">Catch rate</div></div>
                <div className="stat"><div className="n tabular">{s.fish_per_hour == null ? "—" : s.fish_per_hour.toFixed(0)}</div><div className="l">Fish / hour</div></div>
                <div className="stat"><div className="n tabular">{hrs(s.time_fished_sec)}</div><div className="l">Time fished</div></div>
              </div>
              <p className="muted" style={{ margin: "0.6rem 0 0", fontSize: "0.85rem" }}>
                {s.top_zone ? <>Top zone: <b>{s.top_zone.zone}</b></> : "No zone data yet"}
                {s.top_fish ? <> · Top fish: <b>{s.top_fish.label}</b> ×{s.top_fish.n}</> : null}
              </p>
              <p className="faint" style={{ margin: "0.35rem 0 0", fontSize: "0.8rem" }}>
                This week: {s.week.fish} fish / {s.week.casts} casts{ago(s.last_sync) ? ` · ${ago(s.last_sync)}` : ""}
              </p>
            </div>
          ); })()}
          {me.addons.haul && (() => { const h = me.addons.haul; return (
            <div className="panel card lift" role="button" style={{ cursor: "pointer" }}
              onClick={() => api.openExternal(`${siteBase}/me/sessions`)}
              title="Open your sessions on goblineng.co">
              <div className="row between">
                <h3 style={{ margin: 0 }}>💰 Haul</h3>
                <span className="faint" style={{ fontSize: "0.8rem" }}>Sessions ↗</span>
              </div>
              <div className="grid2" style={{ marginTop: "0.5rem" }}>
                <div className="stat"><div className="n tabular">{gold(h.counted)}</div><div className="l">Haul gold</div></div>
                <div className="stat"><div className="n tabular">{h.gold_per_hour == null ? "—" : `${gold(h.gold_per_hour)}/h`}</div><div className="l">Gold / hour</div></div>
                <div className="stat"><div className="n tabular">{gold(h.income)}</div><div className="l">Income</div></div>
                <div className="stat"><div className="n tabular">{h.sessions.toLocaleString()}</div><div className="l">Sessions</div></div>
              </div>
              <p className="muted" style={{ margin: "0.6rem 0 0", fontSize: "0.85rem" }}>
                XP <b>{h.xp_total.toLocaleString()}</b> · Rep <b>{h.rep_total.toLocaleString()}</b> · <b>{h.currency_kinds}</b> currencies
                {h.best_session ? <> · Best session: <b>{gold(h.best_session.counted)}</b></> : null}
              </p>
              <p className="faint" style={{ margin: "0.35rem 0 0", fontSize: "0.8rem" }}>
                This week: {gold(h.week.counted)}{ago(h.last_sync) ? ` · ${ago(h.last_sync)}` : ""}
              </p>
            </div>
          ); })()}
        </div>
      )}

      <div className="panel card" style={{ marginTop: "1rem" }}>
        <h3>What’s new</h3>
        <p className="muted" style={{ margin: 0 }}>{me?.whats_new ?? "Nothing new to report."}</p>
      </div>
    </>
  );
}
