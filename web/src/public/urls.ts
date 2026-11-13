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

/** The public feedback form (#542) and the four pages the API's answer
 * leads to. */
export type FeedbackState = "form" | "sent" | "thanks" | "expired" | "error";

export const FEEDBACK_STATES: readonly FeedbackState[] = [
  "form",
  "sent",
  "thanks",
  "expired",
  "error",
];

export const FEEDBACK_SLUGS: Record<Locale, Record<FeedbackState, string>> = {
  de: {
    form: "feedback",
    sent: "feedback/gesendet",
    thanks: "feedback/danke",
    expired: "feedback/abgelaufen",
    error: "feedback/fehler",
  },
  en: {
    form: "feedback",
    sent: "feedback/sent",
    thanks: "feedback/thanks",
    expired: "feedback/expired",
    error: "feedback/error",
  },
};

export function feedbackUrl(
  locale: Locale,
  state: FeedbackState = "form",
): string {
  return `/${locale}/${FEEDBACK_SLUGS[locale][state]}`;
}

export function otherLocale(locale: Locale): Locale {
  return locale === "de" ? "en" : "de";
}
