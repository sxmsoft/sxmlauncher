/**
 * Turkish is the default UI. English is the second catalog.
 * The choice lives in localStorage — there is no backend language field.
 */

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import en from "./locales/en.json";
import tr from "./locales/tr.json";

const STORAGE_KEY = "sxmlauncher.locale";

export type AppLocale = "tr" | "en";

export function readLocale(): AppLocale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "en" || stored === "tr") return stored;
  } catch {
    // Private mode: Turkish remains the session default.
  }
  return "tr";
}

export function writeLocale(locale: AppLocale): void {
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    // The language still changes for this session.
  }
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  void i18n.changeLanguage(locale);
}

const initial = readLocale();
if (typeof document !== "undefined") {
  document.documentElement.lang = initial;
}

void i18n.use(initReactI18next).init({
  resources: {
    tr: { translation: tr },
    en: { translation: en },
  },
  lng: initial,
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
