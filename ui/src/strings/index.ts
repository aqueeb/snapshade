// `t(key)` — the one lookup every UI layer uses (OPT-06). English ships; a later language
// is another table keyed identically, selected by the Language option.
import { en, type StringKey } from "./en";

export type { StringKey };

let table: Record<StringKey, string> = en;

export function t(key: StringKey): string {
  return table[key];
}

/** Install a translation table (missing keys fall back to English). */
export function setStrings(partial: Partial<Record<StringKey, string>>): void {
  table = { ...en, ...partial };
}
