// Interface language: detection, lookup and application to the static markup.
//
// The language is known synchronously when this module loads (stored choice, otherwise the system
// language), so every module can call `t()` while it is being built. Changing the language stores
// the choice and reloads the window, which is simpler and safer than rebuilding every dynamic part.

import { en, type Key } from "./locales/en";
import { fr } from "./locales/fr";

export type Lang = "en" | "fr";
export type { Key };

/** The languages offered, with their own name (never translated). */
export const LANGUAGES: [Lang, string][] = [
  ["en", "English"],
  ["fr", "Français"],
];

const STORAGE_KEY = "ds5forge.language";
const DICTIONARIES: Record<Lang, Record<Key, string>> = { en, fr };

const isLang = (value: unknown): value is Lang => value === "en" || value === "fr";

function detect(): Lang {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isLang(stored)) return stored;
  } catch {
    // Storage unavailable: fall back to the system language.
  }
  for (const tag of navigator.languages ?? [navigator.language]) {
    const primary = tag.toLowerCase().split("-")[0];
    if (isLang(primary)) return primary;
  }
  return "en";
}

export const lang: Lang = detect();
document.documentElement.lang = lang;

/** The text for `key`, with `{name}` placeholders replaced by `params`. */
export function t(key: Key, params: Record<string, string | number> = {}): string {
  const text = DICTIONARIES[lang][key] ?? en[key];
  return text.replace(/\{(\w+)\}/g, (match, name: string) => (name in params ? String(params[name]) : match));
}

/** The percent sign as written in the language (French puts a space before it). */
export const percent = lang === "fr" ? " %" : "%";

/** Locale tag for date and number formatting. */
export const locale = lang === "fr" ? "fr-FR" : "en-GB";

/** Remembers the language for the next start (and for the other windows, which share the storage). */
export function storeLanguage(value: Lang) {
  try {
    localStorage.setItem(STORAGE_KEY, value);
  } catch {
    // Not remembered: the system language will be used next time.
  }
}

/**
 * Fills the static markup from the dictionary:
 * - `data-i18n="key"`: text content;
 * - `data-i18n-html="key"`: inner HTML (for the few texts with emphasis; the dictionaries are ours);
 * - `data-i18n-attr="attribute:key; attribute:key"`: attributes such as `aria-label` or `title`.
 */
export function applyTranslations(root: ParentNode = document) {
  root.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n as Key);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-html]").forEach((el) => {
    el.innerHTML = t(el.dataset.i18nHtml as Key);
  });
  root.querySelectorAll<HTMLElement>("[data-i18n-attr]").forEach((el) => {
    for (const pair of (el.dataset.i18nAttr ?? "").split(";")) {
      const [attribute, key] = pair.split(":").map((part) => part.trim());
      if (attribute && key) el.setAttribute(attribute, t(key as Key));
    }
  });
}
