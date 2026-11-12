// ONE formatting policy for the whole app.
//
// The rule: everything numeric, temporal or monetary goes through this module,
// and this module formats in the APP LANGUAGE — not the OS locale, and never a
// hardcoded tag.
//
// Why that rule exists: today `Home.tsx` formats a date with
// `toLocaleDateString("en-US", …)` while every number nearby uses a bare
// `toLocaleString()` (the system locale). A German player therefore already sees
// German thousands separators sitting next to an American date, in an English
// UI. Three different locales on one screen. Picking the app language for all of
// it makes the screen internally consistent, and makes the Language setting mean
// what it says.
//
// The formatters are cached: constructing an `Intl.*Format` is expensive and
// these run inside render.

import type { Lang } from "./types";

/// The BCP-47 tag we format with for each shipped language. Deliberately region
/// -less: "en" and "de", not "en-US" and "de-DE". Region belongs to the user, not
/// to the language, and Intl's defaults for the bare tags are the right ones.
const TAG: Record<Lang, string> = { en: "en", de: "de" };

function cached<T>(store: Map<string, T>, key: string, make: () => T): T {
  let v = store.get(key);
  if (v === undefined) {
    v = make();
    store.set(key, v);
  }
  return v;
}

const numberFmts = new Map<string, Intl.NumberFormat>();
const dateFmts = new Map<string, Intl.DateTimeFormat>();
const relFmts = new Map<string, Intl.RelativeTimeFormat>();

/// A plain integer / decimal with the language's grouping and decimal marks.
/// Replaces every bare `n.toLocaleString()` in the views.
export function formatNumber(lang: Lang, n: number, opts?: Intl.NumberFormatOptions): string {
  const key = `${lang}|${JSON.stringify(opts ?? {})}`;
  return cached(numberFmts, key, () => new Intl.NumberFormat(TAG[lang], opts)).format(n);
}

/// A ratio (0..1) as a percentage. Replaces `${Math.round(r * 100)}%`, which
/// hardcodes both the rounding and the "%" placement — and German writes the
/// percent sign with a space before it.
export function formatPercent(lang: Lang, ratio: number, fractionDigits = 0): string {
  return formatNumber(lang, ratio, {
    style: "percent",
    minimumFractionDigits: fractionDigits,
    maximumFractionDigits: fractionDigits,
  });
}

/// A calendar date. Replaces the hardcoded `toLocaleDateString("en-US", …)`.
export function formatDate(lang: Lang, date: Date, opts?: Intl.DateTimeFormatOptions): string {
  const o = opts ?? ({ weekday: "short", month: "short", day: "numeric" } as const);
  const key = `${lang}|${JSON.stringify(o)}`;
  return cached(dateFmts, key, () => new Intl.DateTimeFormat(TAG[lang], o)).format(date);
}

/// A date that arrived as a bare `YYYY-MM-DD` from the server (an ETA). Pinned
/// to midday UTC so a timezone west of Greenwich cannot roll it back a day.
export function formatIsoDate(lang: Lang, isoDay: string, opts?: Intl.DateTimeFormatOptions): string {
  return formatDate(lang, new Date(`${isoDay}T12:00:00Z`), opts);
}

const MINUTE = 60;
const HOUR = 3600;
const DAY = 86400;

/// "5 minutes ago" / "vor 5 Minuten", picking its own unit. Replaces the two
/// hand-rolled `ago()` helpers (`Home.tsx` and `Data.tsx`), which build their
/// output by string concatenation and so cannot be translated at all: German
/// puts the preposition in FRONT ("vor 5 Min."), which no amount of appending
/// " ago" can produce.
export function formatRelativeSeconds(
  lang: Lang,
  secondsAgo: number,
  style: Intl.RelativeTimeFormatStyle = "narrow",
): string {
  const s = Math.max(0, Math.round(secondsAgo));
  const fmt = cached(relFmts, `${lang}|${style}`, () =>
    new Intl.RelativeTimeFormat(TAG[lang], { numeric: "auto", style }),
  );
  if (s < MINUTE) return fmt.format(-s, "second");
  if (s < HOUR) return fmt.format(-Math.round(s / MINUTE), "minute");
  if (s < DAY) return fmt.format(-Math.round(s / HOUR), "hour");
  return fmt.format(-Math.round(s / DAY), "day");
}

/// Seconds since a unix timestamp, or null when there is no timestamp.
export function secondsSince(unixSeconds: number | null | undefined): number | null {
  if (!unixSeconds) return null;
  return Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
}

/// Seconds since an ISO stamp. The server sends `"2026-09-02 11:04:05"` (a
/// space, no zone) in some places, which `Date.parse` treats inconsistently
/// across engines — normalise it here, once, instead of at each call site.
export function secondsSinceIso(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const normalized = iso.includes("T") ? iso : `${iso.replace(" ", "T")}Z`;
  const t = Date.parse(normalized);
  if (Number.isNaN(t)) return null;
  return Math.max(0, (Date.now() - t) / 1000);
}

/// Below this many seconds a stamp reads as "just now" rather than a counted
/// phrase. The two hand-rolled `ago()` helpers this replaces disagreed (Home
/// used 90s, Data used 60s and printed raw seconds below it); one number now.
export const JUST_NOW_SECONDS = 90;

/// THE relative-time helper the views call. `secondsAgo` comes from
/// `secondsSince` / `secondsSinceIso`, so a null (no timestamp) passes straight
/// through and the caller decides what "never" looks like.
///
/// `justNow` is passed in rather than hardcoded because that wording is a
/// dictionary string, not a number format.
export function formatAgo(
  lang: Lang,
  secondsAgo: number | null,
  justNow: string,
  style: Intl.RelativeTimeFormatStyle = "narrow",
): string | null {
  if (secondsAgo == null) return null;
  if (secondsAgo < JUST_NOW_SECONDS) return justNow;
  return formatRelativeSeconds(lang, secondsAgo, style);
}

// ── money ─────────────────────────────────────────────────────────────────────

/// WoW money split into its three coins. The DIGITS are formatted for the
/// language; the coin LETTERS come from the dictionary (`fmt.gold` and friends),
/// because they are client vocabulary, not arithmetic.
///
/// OPEN QUESTION: what the German client prints for the three coins is not
/// something to infer. It is marked `// ASK:` in `de.ts` for the native speaker.
export function splitMoney(copper: number): {
  negative: boolean;
  gold: number;
  silver: number;
  copper: number;
} {
  const negative = copper < 0;
  const abs = Math.abs(Math.trunc(copper));
  return {
    negative,
    gold: Math.floor(abs / 10000),
    silver: Math.floor((abs % 10000) / 100),
    copper: abs % 100,
  };
}

/// A duration split for the `Dur` renderer, same contract as `splitMoney`: this
/// module does the arithmetic, the dictionary supplies the unit letters.
export function splitDuration(seconds: number): { hours: number; minutes: number; seconds: number } {
  const s = Math.max(0, Math.round(seconds));
  return { hours: Math.floor(s / HOUR), minutes: Math.floor((s % HOUR) / MINUTE), seconds: s % MINUTE };
}

/// A countdown split (days / hours / minutes / seconds) for the vote clock.
export function splitCountdown(ms: number): { days: number; hours: number; minutes: number; seconds: number } {
  const t = Math.max(0, Math.floor(ms / 1000));
  return {
    days: Math.floor(t / DAY),
    hours: Math.floor((t % DAY) / HOUR),
    minutes: Math.floor((t % HOUR) / MINUTE),
    seconds: t % MINUTE,
  };
}
