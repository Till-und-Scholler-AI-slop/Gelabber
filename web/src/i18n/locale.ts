// The language Gelabber itself writes in: German or English. It belongs to
// the account (users.locale); signed out, a device keeps the one it last
// showed. index.html picks it before the first paint, startup.ts hands that
// choice to this store, and the session and the `lc` frame keep it in step.

import { useSyncExternalStore } from "react";
import { create } from "zustand";

export type Locale = "de" | "en";

export const LOCALES: readonly Locale[] = ["de", "en"];

/** Where a device remembers the language it last showed. */
export const LOCALE_STORAGE_KEY = "gelabber.locale";

export function isLocale(value: unknown): value is Locale {
  return value === "de" || value === "en";
}

/**
 * The first preferred language that is German or English; English when
 * none is. index.html runs the same rule before the first paint.
 */
export function detectLocale(languages: readonly string[]): Locale {
  for (const tag of languages) {
    const primary = tag.trim().toLowerCase().split(/[-_]/)[0];
    if (isLocale(primary)) return primary;
  }
  return "en";
}

/** The language the interface shows right now. */
export const useLocale = create<{ locale: Locale }>(() => ({ locale: "de" }));

export function currentLocale(): Locale {
  return useLocale.getState().locale;
}

/**
 * The language, for components. Unlike `useLocale(selector)`, a server
 * render (renderToStaticMarkup in tests) sees the current language too,
 * not the store's initial one.
 */
export function useCurrentLocale(): Locale {
  return useSyncExternalStore(
    useLocale.subscribe,
    currentLocale,
    currentLocale,
  );
}

/**
 * Shows the interface in `locale`: the store, the document's `lang`, and
 * (unless `persist` is false) this device's memory of it.
 */
export function setLocale(
  locale: Locale,
  { persist = true }: { persist?: boolean } = {},
): void {
  if (useLocale.getState().locale !== locale) useLocale.setState({ locale });
  if (typeof document !== "undefined") document.documentElement.lang = locale;
  if (!persist) return;
  try {
    localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // Blocked storage: the account still carries the language.
  }
}

/** The language this device remembered, if any. */
export function storedLocale(): Locale | null {
  try {
    const stored = localStorage.getItem(LOCALE_STORAGE_KEY);
    return isLocale(stored) ? stored : null;
  } catch {
    return null;
  }
}

/** The BCP 47 tag `Intl` formats a language with. */
export function intlLocale(locale: Locale): "de-DE" | "en-GB" {
  return locale === "en" ? "en-GB" : "de-DE";
}
