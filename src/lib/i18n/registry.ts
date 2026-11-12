// The dictionary registry. Its own file so `dictionary.ts` (which `de.ts`
// imports for `todo`/`Translation`) never has to import `de.ts` back.

import { en, type EnDictionary } from "./en";
import { de } from "./de";
import type { Translation } from "./dictionary";
import type { Lang } from "./types";

/// Every dictionary, keyed by language tag. Adding a language is: a tag in
/// `LANGS`, a `<tag>.ts` dictionary, and one line here.
export const DICTIONARIES: Record<Lang, EnDictionary | Translation> = { en, de };
