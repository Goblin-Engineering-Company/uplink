// Locale resolution — the frontend half.
//
// ONE mechanism, as decided: a Language setting whose default is "auto". There
// is no separate "follow the system" switch; "auto" IS that switch, and picking
// a language is how you override it.
//
// Resolution order for "auto", most specific first:
//
//   1. the WoW client's own language, read from `<flavor>/WTF/Config.wtf`
//      (`SET textLocale "deDE"`). This is the best signal we have: it is the
//      language the user reads the GAME in, which is the language they want the
//      game's companion app in. Uplink already walks the WTF tree, so this costs
//      one extra file read;
//   2. the OS/browser language list (`navigator.languages`), which in a Tauri
//      webview reflects the operating system's preferred languages;
//   3. English.
//
// Every step here is a pure function taking its inputs, so resolution is
// testable without a WoW install, a webview, or a Tauri command.

import { LANGS, isLang, type Lang, type LangSetting } from "./types";

export const DEFAULT_LANG: Lang = "en";

/// WoW `textLocale` values, as written into Config.wtf by the client. Verified
/// against a real install on 2026-09-02: `_retail_/WTF/Config.wtf` contains
/// `SET textLocale "enUS"` (and `SET audioLocale "enUS"` beside it). We read
/// textLocale — audioLocale can differ (English voices, German text) and the
/// text is what the UI has to match.
export const WOW_LOCALES = [
  "enUS", "enGB", "deDE", "frFR", "esES", "esMX", "itIT",
  "ptBR", "ruRU", "koKR", "zhCN", "zhTW",
] as const;
export type WowLocale = (typeof WOW_LOCALES)[number];

/// WoW locale → an Uplink language, when we ship one. Anything we do not ship
/// returns null, which means "keep looking" rather than "use English": a German
/// client on a machine set to French should still land somewhere sensible.
export function langFromWowLocale(locale: string | null | undefined): Lang | null {
  if (!locale) return null;
  const two = locale.slice(0, 2).toLowerCase();
  return isLang(two) ? two : null;
}

/// A BCP-47 tag (`de`, `de-AT`, `en-GB`) → an Uplink language, or null.
export function langFromBcp47(tag: string | null | undefined): Lang | null {
  if (!tag) return null;
  const primary = tag.split(/[-_]/)[0]?.toLowerCase() ?? "";
  return isLang(primary) ? primary : null;
}

/// First supported language in an ordered preference list.
export function langFromPreferences(tags: readonly string[]): Lang | null {
  for (const tag of tags) {
    const l = langFromBcp47(tag);
    if (l) return l;
  }
  return null;
}

/// The OS languages, as the webview reports them. `navigator.languages` is
/// ordered by preference and is populated in every Tauri v2 webview (WKWebView,
/// WebView2, webkit2gtk), which is why this needs no extra Tauri plugin.
export function systemPreferences(): readonly string[] {
  if (typeof navigator === "undefined") return [];
  const list = navigator.languages;
  if (list && list.length > 0) return list;
  return navigator.language ? [navigator.language] : [];
}

/// What "auto" resolves to, given the signals. Both inputs are optional so a
/// caller with neither still gets a defined answer.
export function detectLang(signals?: {
  /// `textLocale` from the WoW client, e.g. "deDE". Null when no install is
  /// configured, the drive is unplugged, or the file has not been written yet.
  wowLocale?: string | null;
  /// Overrides `navigator.languages`; present for tests.
  systemTags?: readonly string[];
}): Lang {
  return (
    langFromWowLocale(signals?.wowLocale) ??
    langFromPreferences(signals?.systemTags ?? systemPreferences()) ??
    DEFAULT_LANG
  );
}

/// The setting plus the signals gives the language actually in force.
export function resolveLang(
  setting: LangSetting | string | null | undefined,
  signals?: Parameters<typeof detectLang>[0],
): Lang {
  if (typeof setting === "string" && isLang(setting)) return setting;
  return detectLang(signals);
}

/// The options for the Settings picker, in display order: Automatic first, then
/// every shipped language written in its OWN language.
export function languageOptions(): Array<{ value: LangSetting }> {
  return [{ value: "auto" as const }, ...LANGS.map((l) => ({ value: l }))];
}
