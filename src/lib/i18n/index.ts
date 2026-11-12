// The i18n module's public surface. Views import from here and nowhere deeper.
//
//   import { useI18n } from "../lib/i18n";
//   const { t, n, ago } = useI18n();
//   <button>{t("data.sync-now")}</button>
//   <span>{t("data.waiting-chip", { count: queued })}</span>
//
// Design doc: docs/superpowers/specs/2026-09-02-uplink-localization-design.md

export { I18nProvider, useI18n, useT, type I18n } from "./context";
export { translate, makeT, pseudo, type TFunction, type TranslateOptions } from "./translate";
export {
  translateCoreMsg,
  isCoreMsg,
  CORE_MESSAGE_KEYS,
  type CoreMsg,
} from "./msg";
export {
  DEFAULT_LANG,
  WOW_LOCALES,
  detectLang,
  resolveLang,
  langFromWowLocale,
  langFromBcp47,
  langFromPreferences,
  systemPreferences,
  languageOptions,
  type WowLocale,
} from "./locale";
export {
  formatNumber,
  formatPercent,
  formatDate,
  formatIsoDate,
  formatRelativeSeconds,
  formatAgo,
  JUST_NOW_SECONDS,
  secondsSince,
  secondsSinceIso,
  splitMoney,
  splitDuration,
  splitCountdown,
} from "./format";
export {
  translationProgress,
  untranslatedKeys,
  todo,
  todoPlural,
  isTodo,
  stripTodo,
  TODO_MARK,
  type Translation,
} from "./dictionary";
export { DICTIONARIES } from "./registry";
export { en, type EnDictionary, type MsgKey } from "./en";
export { de } from "./de";
export {
  LANGS,
  isLang,
  isLangSetting,
  type Lang,
  type LangSetting,
  type Message,
  type PluralMessage,
  type Dictionary,
  type ParamValue,
} from "./types";
