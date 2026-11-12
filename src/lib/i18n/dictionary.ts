// The translation contract: what a non-English dictionary has to satisfy, and
// how an untranslated string is marked so it can be counted, fallen back on, and
// found by the translator.

import { en, type EnDictionary, type MsgKey } from "./en";
import type { Message, PluralMessage } from "./types";

/// The shape of a translated dictionary. A mapped type over the English keys, so
/// every key is REQUIRED (a missing one is a compile error) and the plural-ness
/// of each entry must match English (a plural in English must be a plural here —
/// a language can add categories, it cannot drop the concept).
export type Translation = {
  [K in MsgKey]: EnDictionary[K] extends string ? string : PluralMessage;
};

/// Marker for "this string has not been translated yet". A private-use codepoint
/// prefix, so it can never collide with real copy and never survives into the UI
/// unnoticed.
export const TODO_MARK = "\uE000";

/// Wrap the English text in a translation file to say "still to do". The value
/// stays a plain string, so the file type-checks exactly like a finished one; at
/// runtime `t()` sees the mark, falls back to English, and counts it as missing.
///
/// A translator's whole job is: search the file for `todo(`, replace
/// `todo("Sync now")` with `"Jetzt synchronisieren"`, repeat. When
/// `translationProgress` reports 0 remaining, the language is done.
export function todo(english: string): string {
  return TODO_MARK + english;
}

/// Same, for a plural entry: every form starts as the English text.
export function todoPlural(forms: PluralMessage): PluralMessage {
  const out: PluralMessage = { other: TODO_MARK + forms.other };
  for (const k of ["zero", "one", "two", "few", "many"] as const) {
    const v = forms[k];
    if (v !== undefined) out[k] = TODO_MARK + v;
  }
  return out;
}

export function isTodo(text: string): boolean {
  return text.startsWith(TODO_MARK);
}

export function stripTodo(text: string): string {
  return isTodo(text) ? text.slice(TODO_MARK.length) : text;
}

/// How much of a language is done. Drives the "some of Uplink isn't translated
/// yet" note in Settings, and is the natural release gate: a language nobody has
/// finished should not be offered as if it were.
export function translationProgress(dict: Translation | EnDictionary): {
  total: number;
  translated: number;
  remaining: number;
  ratio: number;
} {
  const keys = Object.keys(en) as MsgKey[];
  let translated = 0;
  for (const k of keys) {
    const m: Message = dict[k];
    const text = typeof m === "string" ? m : m.other;
    if (!isTodo(text)) translated++;
  }
  const total = keys.length;
  return { total, translated, remaining: total - translated, ratio: total === 0 ? 1 : translated / total };
}

/// Keys still awaiting a translator, in dictionary order. The one report a
/// translator actually needs.
export function untranslatedKeys(dict: Translation): MsgKey[] {
  return (Object.keys(en) as MsgKey[]).filter((k) => {
    const m: Message = dict[k];
    return isTodo(typeof m === "string" ? m : m.other);
  });
}
