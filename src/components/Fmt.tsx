// WoW-standard money + duration renderers — the SAME visual contract as the
// website's Money/Dur components (white digits; true classic coin colors on
// the g/s/c letters; muted h/m/s letters), so gold and time look identical
// in-game, on goblineng.co, and here in Uplink.
import { DEFAULT_LANG, formatNumber } from "../lib/i18n";

// Digit grouping goes through the ONE formatting policy (lib/i18n/format.ts)
// instead of a bare toLocaleString(), which follows the OS locale and so put
// German separators next to an American date on the Home screen. Pinned to
// DEFAULT_LANG until the i18n provider is wired.
const fmtN = (n: number) => formatNumber(DEFAULT_LANG, n);

const COIN = { g: "#ffd70a", s: "#c7c7cf", c: "#eda55f" };
const INK = "var(--color-ink)";
const MUTED = "var(--color-ink-muted)";

export function Money({ copper, compact = true }: { copper: number; compact?: boolean }) {
  const neg = copper < 0;
  const abs = Math.abs(Math.trunc(copper));
  const g = Math.floor(abs / 10000);
  const s = Math.floor((abs % 10000) / 100);
  const c = abs % 100;
  // compact: gold only (cards, headline stats) — white digits, gold "g"
  if (compact) {
    return (
      <span className="tabular" style={{ whiteSpace: "nowrap" }} title={`${fmtN(copper)} copper`}>
        {neg && <span style={{ color: INK }}>−</span>}
        <span style={{ color: INK }}>{fmtN(g)}</span>
        <span style={{ color: COIN.g }}>g</span>
      </span>
    );
  }
  return (
    <span className="tabular" style={{ whiteSpace: "nowrap" }}>
      {neg && <span style={{ color: INK }}>−</span>}
      {g > 0 && <><span style={{ color: INK }}>{fmtN(g)}</span><span style={{ color: COIN.g }}>g</span>{" "}</>}
      {(g > 0 || s > 0) && <><span style={{ color: INK }}>{s}</span><span style={{ color: COIN.s }}>s</span>{" "}</>}
      <span style={{ color: INK }}>{c}</span><span style={{ color: COIN.c }}>c</span>
    </span>
  );
}

export function Dur({ secs }: { secs: number }) {
  const s = Math.max(0, Math.round(secs));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  return (
    <span className="tabular" style={{ whiteSpace: "nowrap" }}>
      {h > 0 && <><span style={{ color: INK }}>{fmtN(h)}</span><span style={{ color: MUTED }}>h</span>{" "}</>}
      {h > 0 || m > 0
        ? <><span style={{ color: INK }}>{m}</span><span style={{ color: MUTED }}>m</span></>
        : <><span style={{ color: INK }}>{s}</span><span style={{ color: MUTED }}>s</span></>}
    </span>
  );
}
