// Where the public pages live (issue #522, "URLs"). Paths are German in
// German and English in English; the landing page is the language folder.

import type { Locale } from "../i18n/locale.ts";

export type LegalPageId = "imprint" | "privacy" | "terms";

/** In the order the navigation and the footer list them. */
export const LEGAL_PAGES: readonly LegalPageId[] = [
  "imprint",
  "privacy",
  "terms",
];

const LEGAL_SLUGS: Record<Locale, Record<LegalPageId, string>> = {
  de: {
    imprint: "impressum",
    privacy: "datenschutz",
    terms: "nutzungsbedingungen",
  },
  en: { imprint: "imprint", privacy: "privacy", terms: "terms" },
};

export function landingUrl(locale: Locale): string {
  return `/${locale}/`;
}

export function legalSlug(locale: Locale, page: LegalPageId): string {
  return LEGAL_SLUGS[locale][page];
}

export function legalUrl(locale: Locale, page: LegalPageId): string {
  return `/${locale}/${legalSlug(locale, page)}`;
}

export function otherLocale(locale: Locale): Locale {
  return locale === "de" ? "en" : "de";
}
