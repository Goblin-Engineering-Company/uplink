// The translation engine. Pure functions — no React, no globals, no I/O — so it
// is trivially testable and can be called from anywhere (including a non-React
// helper such as a formatter or an error mapper).

import { en, type EnDictionary, type MsgKey } from "./en";
import { isTodo, stripTodo, type Translation } from "./dictionary";
import { DICTIONARIES } from "./registry";
import type { Lang, Message, ParamValue, PluralMessage, TArgs } from "./types";

/// Options that change HOW a string is produced, as opposed to which one.
export type TranslateOptions = {
  /// Pseudo-localize the result: expand it by roughly a third and accent the
  /// letters. Nothing to do with any real language — it is how we prove the
  /// layout survives German-length text BEFORE any German exists. See `pseudo`.
  pseudo?: boolean;
};

/// The typed `t`. Note the argument list: a key with no placeholders takes no
/// second argument at all, and a key with placeholders takes an object that must
/// name every one of them. Both are enforced by `TArgs`.
export type TFunction = <K extends MsgKey>(key: K, ...args: TArgs<EnDictionary[K]>) => string;

const dictOf = (lang: Lang): EnDictionary | Translation => DICTIONARIES[lang] ?? en;

/// CLDR plural category for `count` in `lang`. `Intl.PluralRules` is the whole
/// point: it is correct for every language, including the ones with three and
/// four categories that a hand-written `n === 1` check silently gets wrong.
const pluralCache = new Map<string, Intl.PluralRules>();
function pluralRules(lang: Lang): Intl.PluralRules {
  let r = pluralCache.get(lang);
  if (!r) {
    r = new Intl.PluralRules(lang);
    pluralCache.set(lang, r);
  }
  return r;
}

function selectPluralForm(m: PluralMessage, lang: Lang, count: number): string | undefined {
  const category = pluralRules(lang).select(count);
  return m[category] ?? m.other;
}

/// Resolve one key in one language to raw (un-interpolated) text, falling back
/// to English whenever the entry is still marked `todo(...)`. The fallback is
/// PER STRING, not per language: a half-translated German build shows German
/// where German exists and English everywhere else, which is exactly what you
/// want mid-translation and is why a partial dictionary is safe to ship.
function rawText(lang: Lang, key: MsgKey, count: number | undefined): string {
  const source: Message | undefined = (dictOf(lang) as Record<string, Message>)[key];
  const english: Message | undefined = (en as Record<string, Message>)[key];

  // A key that is not in the dictionary at all can only happen at runtime via a
  // dynamically-built key (the Rust code table). Show the key rather than an
  // empty string: silent blanks are the worst failure mode in a translated UI.
  if (source === undefined && english === undefined) {
    if (import.meta.env?.DEV) console.warn(`[i18n] unknown key: ${key}`);
    return key;
  }

  const pickFrom = (m: Message | undefined, l: Lang): string | undefined => {
    if (m === undefined) return undefined;
    if (typeof m === "string") return m;
    return selectPluralForm(m, l, count ?? 0);
  };

  const primary = pickFrom(source, lang);
  if (primary !== undefined && !isTodo(primary)) return primary;

  const fallback = pickFrom(english, "en");
  if (fallback !== undefined) return fallback;
  return primary !== undefined ? stripTodo(primary) : key;
}

const PLACEHOLDER = /\{(\w+)\}/g;

function interpolate(text: string, params: Record<string, ParamValue> | undefined): string {
  if (!params) return text;
  return text.replace(PLACEHOLDER, (whole, name: string) => {
    const v = params[name];
    if (v === undefined) {
      if (import.meta.env?.DEV) console.warn(`[i18n] missing parameter "${name}" in: ${text}`);
      return whole;
    }
    return String(v);
  });
}

/// Pseudo-localization: same words, ~35% longer, visibly accented, wrapped in
/// brackets so a truncated string is obvious. It exists to answer "does the
/// layout hold when the text gets longer" without waiting on a translator, and
/// it catches concatenated / hardcoded strings too: anything that comes out
/// unaccented never went through `t()`.
const ACCENTS: Record<string, string> = {
  a: "á", e: "é", i: "í", o: "ó", u: "ú", n: "ñ", c: "ç", s: "š", y: "ý",
  A: "Á", E: "É", I: "Í", O: "Ó", U: "Ú", N: "Ñ", C: "Ç", S: "Š", Y: "Ý",
};
export function pseudo(text: string): string {
  const accented = text.replace(/[a-zA-Z]/g, (ch) => ACCENTS[ch] ?? ch);
  const padding = "·".repeat(Math.max(1, Math.ceil(text.length * 0.35)));
  return `[${accented}${padding}]`;
}

/// The one entry point. `params` is untyped here because this is the raw engine;
/// the typing lives in `TFunction`, which every caller uses.
export function translate(
  lang: Lang,
  key: MsgKey,
  params?: Record<string, ParamValue>,
  opts?: TranslateOptions,
): string {
  const count = typeof params?.count === "number" ? params.count : undefined;
  const out = interpolate(rawText(lang, key, count), params);
  return opts?.pseudo ? pseudo(out) : out;
}

/// Build a bound, typed `t` for a language. This is what the React context hands
/// out, and what a non-React caller should use.
export function makeT(lang: Lang, opts?: TranslateOptions): TFunction {
  return ((key: MsgKey, params?: Record<string, ParamValue>) =>
    translate(lang, key, params, opts)) as TFunction;
}
