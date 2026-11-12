import { useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import type { Ctx } from "../App";
import { api } from "../lib/api";
import type { Me } from "../lib/types";
import { Money, Dur } from "../components/Fmt";
import {
  DEFAULT_LANG,
  formatAgo,
  formatIsoDate,
  formatNumber,
  formatPercent,
  secondsSinceIso,
} from "../lib/i18n";

// Money/Dur come from components/Fmt — the WoW-standard white-digit +
// coin-color rendering shared with the website (server sends copper/seconds).
// ONE formatting policy (i18n design §7): every number, percent and date on
// this screen goes through lib/i18n/format.ts in the app language. Until the
// provider is wired (next phase) that language is DEFAULT_LANG, so the rendered
// output is unchanged, but nothing here hardcodes "en-US" any more.
const LANG = DEFAULT_LANG;
const pct = (r: number | null) => (r == null ? "—" : formatPercent(LANG, r));
// One votable row — a strict SINGLE line: rank, title (ellipsized, never
// wraps), a compact ✓ marker when this user's weight sits here, and the
// right-aligned vote count. The whole row is the click target.
function VoteRow({ v, rank }: { v: { number: number; title: string; url: string; votes: number; my_votes?: number; my_boosts?: number }; rank: number }) {
  const mine = (v.my_votes ?? 0) > 0;
  const boosts = v.my_boosts ?? 0;
  return (
    <div className="vote-row" onClick={() => api.openExternal(v.url)}
      title={`#${v.number} ${v.title} · open on goblineng.co`}>
      <span className="vote-rank tabular">{rank}</span>
      <span className="vote-title">{v.title}</span>
      {mine && (
        <span className="vote-mine" title={boosts > 0 ? `Your vote is here, including ${boosts} boost${boosts === 1 ? "" : "s"}` : "Your free vote is here"}>
          ✓{boosts > 0 ? ` ⚡${boosts}` : ""}
        </span>
      )}
      <span className="vote-count tabular">{v.votes}</span>
    </div>
  );
}

// The release-race header inside the votes card (spec 2026-07-27), two
// clearly-separated pieces:
//  1. the CLOCK — a button-like tile with a live d/h/m/s countdown. The
//     server SEEDS the deadline (votes_close_at); the tile ticks itself every
//     second locally, so it never needs a sync to move — a refresh merely
//     reseeds it if anything drifted.
//  2. the WORKSHOP row — "In the workshop" label chip on the left, the item
//     that WON the community vote in the middle (ellipsized), expected
//     release on the far right. Reads as "this is being built", not "I voted
//     for this".
function RaceBanner({ race, freeVote, siteBase }: {
  race: NonNullable<Me["race"]>; freeVote: boolean; siteBase: string;
}) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(t);
  }, []);
  const remainMs = race.votes_close_at ? Date.parse(race.votes_close_at) - now : 0;
  // The clock hitting zero locally counts as closed even before the next
  // server refresh confirms it — the tile flips to "tallying" on its own.
  const closed = race.votes_closed || (!!race.votes_close_at && remainMs <= 0);
  const open = !!race.votes_close_at && !closed;
  if (!open && !closed && !race.building) return null;

  // Same visual contract as Money/Dur everywhere else: WHITE digits, the
  // unit letters muted.
  const cd = (ms: number) => {
    const t = Math.max(0, Math.floor(ms / 1000));
    const d = Math.floor(t / 86400), h = Math.floor((t % 86400) / 3600), m = Math.floor((t % 3600) / 60), sec = t % 60;
    const pad = (n: number) => String(n).padStart(2, "0");
    const parts: Array<[string, string]> = [];
    if (d > 0) parts.push([String(d), "d"]);
    if (d > 0 || h > 0) parts.push([d > 0 ? pad(h) : String(h), "h"]);
    parts.push([parts.length ? pad(m) : String(m), "m"]);
    parts.push([pad(sec), "s"]);
    return parts.map(([n, u], i) => (
      <span key={u}>
        {i > 0 && " "}
        <span style={{ color: "var(--color-ink)" }}>{n}</span>
        <span style={{ color: "var(--color-ink-muted)" }}>{u}</span>
      </span>
    ));
  };
  const fmtEta = (d: string) => formatIsoDate(LANG, d);

  return (
    <div className="stack" style={{ gap: "0.55rem" }}>
      {race.votes_close_at && (
        <div className="race-clock">
          <div className="kicker" style={{ fontSize: "0.66rem" }}>{closed ? "Voting closed" : "Voting closes in"}</div>
          {closed ? (
            <div className="race-clock-time" style={{ fontSize: "1.05rem" }}>tallying the votes…</div>
          ) : (
            <div className="race-clock-time tabular">{cd(remainMs)}</div>
          )}
          <div className="faint" style={{ fontSize: "0.72rem" }}>
            {race.votes_cast} vote{race.votes_cast === 1 ? "" : "s"} cast
            {open && freeVote ? " · your free vote expires with this race" : ""}
          </div>
        </div>
      )}
      {race.building && (
        <>
          <div className="workshop-row">
            <span className="chip chip-brass" style={{ flex: "none" }}>In the workshop</span>
            <a className="workshop-title" title={`${race.building.title} · open on goblineng.co`}
              onClick={() => api.openExternal(`${siteBase}/roadmap/${race.building!.number}`)}>
              {race.building.title} ↗
            </a>
            {race.building.eta_date && <span className="workshop-eta">{fmtEta(race.building.eta_date)}</span>}
          </div>
          {race.building.eta_note && (
            <div className="faint" style={{ fontSize: "0.76rem", paddingLeft: "0.2rem" }}>{race.building.eta_note}</div>
          )}
        </>
      )}
    </div>
  );
}

// "synced 5m ago" / "synced just now", built by Intl.RelativeTimeFormat
// rather than by appending " ago" — German puts the preposition in FRONT
// ("vor 5 Min."), which string concatenation can never produce. Shares ONE
// helper with the Data page.
const ago = (iso: string | null) => {
  const rel = formatAgo(LANG, secondsSinceIso(iso), "just now");
  return rel == null ? null : `synced ${rel}`;
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
  }, [ctx.syncNonce]);

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
          {/* labeled rule up top — breathing room instead of a crammed header */}
          <div className="cog-rule" style={{ margin: "0.1rem 0 0.8rem" }}>
            <span className="kicker">The drawing board</span>
          </div>

          {/* the race clock: countdown / tallying / voted-in */}
          {me?.race && (
            <RaceBanner race={me.race} freeVote={!!me.vote_state?.free_vote_available} siteBase={siteBase} />
          )}

          {/* your ballot, on its own line */}
          {me?.vote_state && (
            <div className="row" style={{ gap: "0.4rem", margin: "0.6rem 0 0.2rem", flexWrap: "wrap" }}>
              <span className="faint" style={{ fontSize: "0.78rem" }}>Your ballot:</span>
              <span className={`chip ${me.vote_state.free_vote_available ? "chip-green" : ""}`}
                title={me.vote_state.free_vote_available ? "You have a free vote this cycle. Spend it." : "Free vote cast this cycle"}>
                {me.vote_state.free_vote_available ? "1 free vote" : "free vote cast ✓"}
              </span>
              <span className="chip chip-spark" title="Boost credits: spend them as extra votes. Refunded on anything that doesn't ship.">
                ⚡ {me.vote_state.boost_credits}
              </span>
            </div>
          )}

          {/* the standings — strict one-line rows */}
          {(me?.votables ?? []).length === 0 ? (
            <p className="muted" style={{ marginTop: "0.6rem" }}>No open votes right now.</p>
          ) : (
            <>
              <div className="vote-head">
                <span>Top votes</span>
                <span>votes</span>
              </div>
              <div className="vote-list">
                {me!.votables.slice(0, 5).map((v, i) => <VoteRow key={v.number} v={v} rank={i + 1} />)}
              </div>
              {me!.votables.length > 5 && (
                <details>
                  <summary className="faint" style={{ cursor: "pointer", fontSize: "0.8rem", padding: "0.4rem 0.2rem" }}>
                    +{me!.votables.length - 5} more
                  </summary>
                  <div className="vote-list">
                    {me!.votables.slice(5).map((v, i) => <VoteRow key={v.number} v={v} rank={i + 6} />)}
                  </div>
                </details>
              )}
            </>
          )}

          {/* one CTA, bottom right */}
          <div className="row" style={{ justifyContent: "flex-end", marginTop: "0.7rem" }}>
            <button className="btn-brass btn-sm" onClick={() => api.openExternal(`${siteBase}/vote`)}>
              Cast your vote ↗
            </button>
          </div>
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
                <h3 style={{ margin: 0 }}>🎣 SBF</h3>
                <span className="faint" style={{ fontSize: "0.8rem" }}>Fishing stats ↗</span>
              </div>
              <div className="grid2" style={{ marginTop: "0.5rem" }}>
                <div className="stat"><div className="n tabular">{formatNumber(LANG, s.fish)}</div><div className="l">Fish caught</div></div>
                <div className="stat"><div className="n tabular">{pct(s.catch_rate)}</div><div className="l">Catch rate</div></div>
                <div className="stat"><div className="n tabular">{s.fish_per_hour == null ? "—" : formatNumber(LANG, s.fish_per_hour, { maximumFractionDigits: 0 })}</div><div className="l">Fish / hour</div></div>
                <div className="stat"><div className="n tabular"><Dur secs={s.time_fished_sec} /></div><div className="l">Time fished</div></div>
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
                <div className="stat"><div className="n tabular"><Money copper={h.counted} /></div><div className="l">Haul gold</div></div>
                <div className="stat"><div className="n tabular">{h.gold_per_hour == null ? "—" : <><Money copper={h.gold_per_hour} /><span className="muted">/h</span></>}</div><div className="l">Gold / hour</div></div>
                <div className="stat"><div className="n tabular"><Money copper={h.income} /></div><div className="l">Income</div></div>
                <div className="stat"><div className="n tabular">{formatNumber(LANG, h.sessions)}</div><div className="l">Sessions</div></div>
              </div>
              <p className="muted" style={{ margin: "0.6rem 0 0", fontSize: "0.85rem" }}>
                XP <b>{formatNumber(LANG, h.xp_total)}</b> · Rep <b>{formatNumber(LANG, h.rep_total)}</b> · <b>{h.currency_kinds}</b> currencies
                {h.best_session ? <> · Best session: <b><Money copper={h.best_session.counted} /></b></> : null}
              </p>
              <p className="faint" style={{ margin: "0.35rem 0 0", fontSize: "0.8rem" }}>
                This week: <Money copper={h.week.counted} />{ago(h.last_sync) ? ` · ${ago(h.last_sync)}` : ""}
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
