// The operator values and blocks of a public page's template. The web
// container fills them in at start
// (web/docker-entrypoint.d/50-gelabber-public-pages.sh): a value is written
// as `{{KEY}}` and HTML-escaped when it is filled in; a block (Template.tsx)
// is kept only while its key is not empty.

/** Operator values, and what the container works out from them. */
export const KEY = {
  operatorName: "GELABBER_OPERATOR_NAME",
  operatorStreet: "GELABBER_OPERATOR_STREET",
  operatorCity: "GELABBER_OPERATOR_CITY",
  operatorEmail: "GELABBER_OPERATOR_EMAIL",
  operatorPhone: "GELABBER_OPERATOR_PHONE",
  contentResponsible: "GELABBER_CONTENT_RESPONSIBLE",
  reportEmail: "GELABBER_REPORT_EMAIL",
  contactEmail: "GELABBER_CONTACT_EMAIL",
  sourceUrl: "GELABBER_SOURCE_URL",
  guideUrl: "GELABBER_OPERATOR_GUIDE_URL",
  publicUrl: "APP_PUBLIC_URL",
  // GELABBER_INVITE_REQUEST as an https:// URL, or as an email address
  // (else GELABBER_CONTACT_EMAIL); at most one of the two is set.
  inviteUrl: "INVITE_URL",
  inviteEmail: "INVITE_EMAIL",
  // Set while that legal page is published.
  imprint: "PAGE_IMPRINT",
  privacy: "PAGE_PRIVACY",
  terms: "PAGE_TERMS",
  // Set while the public feedback form is published (#542).
  feedback: "PAGE_FEEDBACK",
  // The language of the operator's text on this page, and whether it is
  // the other language's because this one has none.
  fragmentLang: "FRAGMENT_LANG",
  onlyOtherLanguage: "ONLY_OTHER_LANGUAGE",
} as const;

export type TemplateKey = (typeof KEY)[keyof typeof KEY];

export function value(key: TemplateKey): string {
  return `{{${key}}}`;
}

/** For text written into a template outside React. */
export function escapeHtml(text: string): string {
  return text
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
