// The pages of the operator area, by their (German) URL slug:
// `/betrieb/<page>`. The order is the sidebar's.

export const OPERATOR_PAGES = [
  "einladungen",
  "konten",
  "server",
  "meldungen",
  "auslastung",
] as const;

export type OperatorPage = (typeof OPERATOR_PAGES)[number];

/** Where `/betrieb` and the user menu's "Betrieb" lead. */
export const DEFAULT_OPERATOR_PAGE: OperatorPage = "server";

export function isOperatorPage(value: unknown): value is OperatorPage {
  return (OPERATOR_PAGES as readonly unknown[]).includes(value);
}
