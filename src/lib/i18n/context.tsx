// The React surface: one provider near the root, one hook everywhere else.

import { createContext, useContext, useMemo, type ReactNode } from "react";
import type { EnDictionary, MsgKey } from "./en";
import { DEFAULT_LANG, resolveLang } from "./locale";
import { makeT, translate, type TFunction } from "./translate";
import {
  formatDate,
  formatIsoDate,
  formatNumber,
  formatPercent,
  formatRelativeSeconds,
} from "./format";
import { translateCoreMsg, type CoreMsg } from "./msg";
import type { Lang, LangSetting, ParamValue, TArgs } from "./types";

export type I18n = {
  /// The language actually in force (never "auto").
  lang: Lang;
  /// The setting as stored, so Settings can show "Automatic" as selected.
  setting: LangSetting;
  /// Translate to a string.
  t: TFunction;
  /// Translate to React nodes, for a sentence with markup inside it.
  tx: <K extends MsgKey>(key: K, params: Record<string, ReactNode | ParamValue>) => ReactNode;
  /// A message that came from Rust as `{ code, params }`.
  tc: (msg: CoreMsg | string | null | undefined) => string | null;
  /// Formatting, bound to the same language as the words (format.ts).
  n: (value: number, opts?: Intl.NumberFormatOptions) => string;
  pct: (ratio: number, fractionDigits?: number) => string;
  date: (date: Date, opts?: Intl.DateTimeFormatOptions) => string;
  isoDate: (isoDay: string, opts?: Intl.DateTimeFormatOptions) => string;
  ago: (secondsAgo: number, style?: Intl.RelativeTimeFormatStyle) => string;
};

// The default context is a working English instance rather than `null`. A view
// rendered outside the provider (a test, a Storybook-style harness, the error
// screen that renders before config loads) then shows English instead of
// throwing, which is the right failure mode for a display concern.
const I18nContext = createContext<I18n | null>(null);

/// Split a translated string on its `{placeholders}` and substitute React nodes,
/// for the sentences that carry markup: "Pair this device in <b>Settings</b> to
/// see your broadcast" is ONE sentence and must stay one dictionary entry —
/// German puts "Settings" somewhere else in it. The dictionary holds
/// "Pair this device in {settings} to see…"; the view passes
/// `{ settings: <b>{t("nav.settings")}</b> }`.
///
/// This is why a sentence must never be assembled from fragments in JSX.
function interpolateNodes(
  text: string,
  params: Record<string, ReactNode | ParamValue>,
): ReactNode {
  const parts: ReactNode[] = [];
  const re = /\{(\w+)\}/g;
  let last = 0;
  let m: RegExpExecArray | null;
  let i = 0;
  while ((m = re.exec(text)) !== null) {
    if (m.index > last) parts.push(text.slice(last, m.index));
    const value = params[m[1]];
    parts.push(
      typeof value === "string" || typeof value === "number" ? (
        String(value)
      ) : value === undefined ? (
        m[0]
      ) : (
        <span key={`p${i}`}>{value}</span>
      ),
    );
    last = m.index + m[0].length;
    i++;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}

export function I18nProvider({
  setting,
  wowLocale,
  pseudo,
  children,
}: {
  /// The stored Language setting. "auto" (the default) runs detection.
  setting?: LangSetting;
  /// `textLocale` from the WoW client's Config.wtf, when the core has read it.
  wowLocale?: string | null;
  /// Pseudo-localize every string. Dev-only layout check; see translate.ts.
  pseudo?: boolean;
  children: ReactNode;
}) {
  const value = useMemo<I18n>(() => {
    const lang = resolveLang(setting ?? "auto", { wowLocale });
    const t = makeT(lang, { pseudo });
    return {
      lang,
      setting: setting ?? "auto",
      t,
      tx: (key, params) => {
        // `count` still has to reach the engine so a plural entry picks its
        // form; every other placeholder is left in the text for the node pass.
        const count = typeof params.count === "number" ? { count: params.count } : undefined;
        return interpolateNodes(translate(lang, key, count, { pseudo }), params);
      },
      tc: (msg) => translateCoreMsg(lang, msg),
      n: (value_, opts) => formatNumber(lang, value_, opts),
      pct: (ratio, fractionDigits) => formatPercent(lang, ratio, fractionDigits),
      date: (date, opts) => formatDate(lang, date, opts),
      isoDate: (isoDay, opts) => formatIsoDate(lang, isoDay, opts),
      ago: (secondsAgo, style) => formatRelativeSeconds(lang, secondsAgo, style),
    };
  }, [setting, wowLocale, pseudo]);

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

const FALLBACK: I18n = (() => {
  const lang = DEFAULT_LANG;
  const t = makeT(lang);
  return {
    lang,
    setting: "auto",
    t,
    tx: (key, params) => interpolateNodes(translate(lang, key), params),
    tc: (msg) => translateCoreMsg(lang, msg),
    n: (v, opts) => formatNumber(lang, v, opts),
    pct: (r, d) => formatPercent(lang, r, d),
    date: (d, opts) => formatDate(lang, d, opts),
    isoDate: (d, opts) => formatIsoDate(lang, d, opts),
    ago: (s, style) => formatRelativeSeconds(lang, s, style),
  };
})();

/// The hook every view uses: `const { t } = useI18n();`
export function useI18n(): I18n {
  return useContext(I18nContext) ?? FALLBACK;
}

/// Sugar for the common case.
export function useT(): TFunction {
  return useI18n().t;
}

// Re-exported so a view importing the hook has the arg types available too.
export type { EnDictionary, MsgKey, TArgs };
