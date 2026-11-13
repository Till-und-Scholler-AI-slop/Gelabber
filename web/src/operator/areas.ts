// The pages of the operator area, by their (German) URL slug:
// `/betrieb/<page>`. The order is the sidebar's.

export const OPERATOR_PAGES = [
  "einladungen",
  "konten",
  "server",
  "meldungen",
  "feedback",
  "auslastung",
] as const;

export type OperatorPage = (typeof OPERATOR_PAGES)[number];

/** Where `/betrieb` and the user menu's "Betrieb" lead. */
export const DEFAULT_OPERATOR_PAGE: OperatorPage = "server";

export function isOperatorPage(value: unknown): value is OperatorPage {
  return (OPERATOR_PAGES as readonly unknown[]).includes(value);
}

/**
 * `?q=`: what a page's search starts with ("Zum Server" from Meldungen).
 * `?entry=`: the feedback entry an operator mail links to.
 */
export type OperatorSearch = { q?: string; entry?: string };

/** The API takes searches of up to 100 characters. */
export const SEARCH_MAX = 100;

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function validateOperatorSearch(
  search: Record<string, unknown>,
): OperatorSearch {
  const q = typeof search.q === "string" ? search.q.trim() : "";
  const entry =
    typeof search.entry === "string" && UUID.test(search.entry)
      ? search.entry.toLowerCase()
      : "";
  return {
    ...(q ? { q: q.slice(0, SEARCH_MAX) } : {}),
    ...(entry ? { entry } : {}),
  };
}
