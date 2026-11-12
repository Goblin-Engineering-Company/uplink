// Self-checks for the i18n module.
//
// Two halves, both cheap:
//
//   * TYPE assertions, which `tsc --noEmit` (already the first step of
//     `npm run build`) enforces on every build. These are the guarantees that
//     matter most — a missing German key, or a `t()` call that forgot a
//     placeholder, has to fail the BUILD, not a test somebody can skip.
//   * a runtime `runI18nSelfTest()` returning a list of failures, so the same
//     checks can be wired into whatever test runner the repo grows (there is no
//     JS test runner in `uplink/package.json` today — only `cargo test` for the
//     Rust side) or run from a script.

import { en, type MsgKey } from "./en";
import { de } from "./de";
import { DICTIONARIES } from "./registry";
import { translationProgress, untranslatedKeys, isTodo } from "./dictionary";
import { translate } from "./translate";
import { detectLang, langFromWowLocale, resolveLang } from "./locale";
import { formatNumber, formatPercent, splitMoney } from "./format";
import { translateCoreMsg, CORE_MESSAGE_KEYS } from "./msg";
import { LANGS, type Message, type ParamValue, type TArgs } from "./types";

// ── type-level assertions ─────────────────────────────────────────────────────

export type Expect<T extends true> = T;
export type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2 ? true : false;

/// Every English key exists in German, and vice versa. `de.ts` is typed as
/// `Translation`, so this is already enforced at its declaration; asserting it
/// here as well makes the intent explicit and catches a future dictionary added
/// without that annotation.
export type _KeysMatch = Expect<Equal<keyof typeof en, keyof typeof de>>;

/// `t("nav.home")` takes NO second argument.
export type _NoArgs = Expect<Equal<TArgs<(typeof en)["nav.home"]>, []>>;

/// `t("setup.step", …)` requires exactly the placeholders the English text uses.
export type _StepArgs = Expect<
  Equal<TArgs<(typeof en)["setup.step"]>, [params: { n: ParamValue; total: ParamValue }]>
>;

/// A plural entry always requires `count`, whether or not the text prints it.
export type _PluralArgs = Expect<
  Equal<TArgs<(typeof en)["home.votes-cast"]>, [params: { count: ParamValue }]>
>;

// ── runtime checks ────────────────────────────────────────────────────────────

export type SelfTestResult = { name: string; ok: boolean; detail?: string };

export function runI18nSelfTest(): SelfTestResult[] {
  const out: SelfTestResult[] = [];
  const check = (name: string, ok: boolean, detail?: string) => out.push({ name, ok, detail });

  // 1. Same key set in every dictionary, and matching plural-ness.
  const enKeys = Object.keys(en) as MsgKey[];
  for (const lang of LANGS) {
    const dict = DICTIONARIES[lang] as Record<string, Message>;
    const missing = enKeys.filter((k) => dict[k] === undefined);
    check(`${lang}: no missing keys`, missing.length === 0, missing.slice(0, 5).join(", "));

    const extra = Object.keys(dict).filter((k) => !(k in en));
    check(`${lang}: no extra keys`, extra.length === 0, extra.slice(0, 5).join(", "));

    const shapeMismatch = enKeys.filter((k) => {
      const a: Message = (en as Record<string, Message>)[k];
      const b: Message = dict[k];
      return b !== undefined && typeof a !== typeof b;
    });
    check(`${lang}: plural shape matches English`, shapeMismatch.length === 0, shapeMismatch.join(", "));
  }

  // 2. A translated string may not invent or lose a placeholder. A German
  //    sentence may MOVE {name}, but dropping it silently blanks a value.
  const placeholders = (s: string) => new Set(s.match(/\{(\w+)\}/g) ?? []);
  const mismatched: string[] = [];
  for (const k of enKeys) {
    const a: Message = (en as Record<string, Message>)[k];
    const b: Message = (de as Record<string, Message>)[k];
    const at = typeof a === "string" ? a : a.other;
    const bt = typeof b === "string" ? b : b.other;
    if (isTodo(bt)) continue; // not translated yet — it IS the English text
    const pa = placeholders(at);
    const pb = placeholders(bt);
    if (pa.size !== pb.size || [...pa].some((p) => !pb.has(p))) mismatched.push(k);
  }
  check("de: placeholders preserved", mismatched.length === 0, mismatched.slice(0, 5).join(", "));

  // 3. Every core code maps to a key that exists.
  const badCodes = Object.entries(CORE_MESSAGE_KEYS).filter(([, key]) => !(key in en));
  check("core codes map to real keys", badCodes.length === 0, badCodes.map(([c]) => c).join(", "));

  // 4. An untranslated German string falls back to English, per string.
  const fellBack = translate("de", "data.sync-now");
  check("de falls back to English per string", fellBack === en["data.sync-now"], fellBack);

  // 5. Plurals go through Intl.PluralRules, not `n === 1`.
  check("plural: one", translate("en", "home.votes-cast", { count: 1 }) === "1 vote cast");
  check("plural: other", translate("en", "home.votes-cast", { count: 5 }) === "5 votes cast");
  check("plural: zero uses other", translate("en", "home.votes-cast", { count: 0 }) === "0 votes cast");

  // 6. Interpolation.
  check(
    "interpolation",
    translate("en", "setup.step", { n: 2, total: 3 }) === "Step 2 of 3",
    translate("en", "setup.step", { n: 2, total: 3 }),
  );

  // 7. Locale resolution order.
  check("wow locale wins", detectLang({ wowLocale: "deDE", systemTags: ["en-US"] }) === "de");
  check("system used when wow unknown", detectLang({ wowLocale: null, systemTags: ["de-AT"] }) === "de");
  check("unsupported wow locale keeps looking", detectLang({ wowLocale: "frFR", systemTags: ["de-DE"] }) === "de");
  check("falls back to English", detectLang({ wowLocale: "koKR", systemTags: ["ja-JP"] }) === "en");
  check("explicit setting overrides detection", resolveLang("en", { wowLocale: "deDE" }) === "en");
  check("verified real value: enUS", langFromWowLocale("enUS") === "en");

  // 8. Formatting follows the app language, not the system.
  check("de numbers", formatNumber("de", 1234.5) === "1.234,5", formatNumber("de", 1234.5));
  check("en numbers", formatNumber("en", 1234.5) === "1,234.5", formatNumber("en", 1234.5));
  check("percent formats", formatPercent("en", 0.42) === "42%", formatPercent("en", 0.42));

  // 9. Money split (arithmetic only; the coin letters are dictionary entries).
  const m = splitMoney(123456);
  check("money split", m.gold === 12 && m.silver === 34 && m.copper === 56, JSON.stringify(m));

  // 10. Core messages.
  check(
    "core message renders from a code",
    translateCoreMsg("en", { code: "wow_running" }) === en["core.wow-running"],
  );
  check(
    "unknown core code falls back to the core's English text",
    translateCoreMsg("en", { code: "invented_later", text: "something new" }) === "something new",
  );
  check("legacy bare string passes through", translateCoreMsg("en", "a sync is already running") === "a sync is already running");

  return out;
}

/// Human-readable translation status, for the report a translator works from.
export function translationReport(): string {
  const p = translationProgress(de);
  const remaining = untranslatedKeys(de);
  return [
    `de: ${p.translated}/${p.total} translated (${Math.round(p.ratio * 100)}%), ${p.remaining} remaining`,
    ...remaining.slice(0, 20).map((k) => `  todo ${k}`),
    remaining.length > 20 ? `  … and ${remaining.length - 20} more` : "",
  ]
    .filter(Boolean)
    .join("\n");
}
