// The i18n type machinery. Everything here is compile-time only — no runtime cost.
//
// The design in one line: `en.ts` is the SOURCE OF TRUTH (declared `as const`, so
// every message keeps its literal type), and every other language is typed as a
// mapped type over it. That buys three static guarantees:
//
//   1. a key missing from `de.ts` is a TypeScript ERROR (mapped type, all keys required),
//   2. a key that exists ONLY in `de.ts` is a TypeScript ERROR (excess property check),
//   3. `t("some.key", { … })` errors when a placeholder the English text uses is
//      not supplied, or when a placeholder that doesn't exist is passed.
//
// A half-translated release therefore cannot compile, let alone ship.

/// Every language Uplink ships. Adding one = add the tag here + a dictionary file.
export const LANGS = ["en", "de"] as const;
export type Lang = (typeof LANGS)[number];

/// The language SETTING, as stored in config: an explicit language, or automatic
/// detection (see locale.ts). This is the one mechanism — there is no second
/// "follow the system" checkbox.
export type LangSetting = Lang | "auto";

export function isLang(v: string): v is Lang {
  return (LANGS as readonly string[]).includes(v);
}

export function isLangSetting(v: string): v is LangSetting {
  return v === "auto" || isLang(v);
}

/// A plural message. Keyed by CLDR plural categories, which we resolve at runtime
/// with `Intl.PluralRules` — NOT by a hand-rolled `n === 1 ? a : b`. English and
/// German both happen to be one/other languages, but Polish and Russian are not,
/// and a binary check bakes an English assumption into the engine that a future
/// translator cannot fix from a dictionary file.
///
/// `other` is required because CLDR guarantees it exists in every language; the
/// rest are optional and fall back to it.
export type PluralMessage = {
  other: string;
  zero?: string;
  one?: string;
  two?: string;
  few?: string;
  many?: string;
};

/// One dictionary entry: a plain string, or a plural set.
export type Message = string | PluralMessage;

/// A dictionary is a flat map of dotted keys to messages. Flat, not nested: a
/// nested tree reads nicely but makes the "missing key" error point at the wrong
/// object, and `Ctrl-F "data.sync.title"` stops working.
export type Dictionary = Readonly<Record<string, Message>>;

// ── placeholder extraction ────────────────────────────────────────────────────
// Interpolation syntax is `{name}`. We pull the names out of the ENGLISH literal
// at the type level, so the call site is checked against the source text.

/// Union of the `{placeholder}` names inside a string literal type.
export type Placeholder<S extends string> =
  S extends `${string}{${infer P}}${infer Rest}` ? P | Placeholder<Rest> : never;

/// What an interpolated value may be. Kept to primitives on purpose — anything
/// that has to be FORMATTED (a date, a count of gold, a percentage) goes through
/// format.ts first, so formatting is never accidentally locale-blind.
export type ParamValue = string | number;

/// The English source text for a key — for a plural entry, its `other` form,
/// which is the canonical text every other form is a variation of.
export type SourceText<M extends Message> =
  M extends string ? M : M extends { other: infer O extends string } ? O : never;

/// The parameter names a key requires: its placeholders, plus `count` when it is
/// a plural entry (a plural message is meaningless without the number).
export type ParamNames<M extends Message> =
  M extends string ? Placeholder<M> : Placeholder<SourceText<M>> | "count";

/// The argument list for `t(key, …)`: no second argument at all when the message
/// takes no parameters, a required and exhaustively-typed object when it does.
export type TArgs<M extends Message> = [ParamNames<M>] extends [never]
  ? []
  : [params: { [K in ParamNames<M>]: ParamValue }];
