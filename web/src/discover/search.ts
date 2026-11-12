// `/discover?q=…&lang=…`: the search and the language filter live in the
// address, so a reload or a shared link keeps them.

import { QUERY_MAX } from "./rules.ts";

export type DiscoverSearch = { q?: string; lang?: string };

const LANGUAGE = /^[a-z]{2}$/;

export function validateDiscoverSearch(
  search: Record<string, unknown>,
): DiscoverSearch {
  const out: DiscoverSearch = {};
  // The router reads an all-digit search as a number.
  const raw = typeof search.q === "number" ? String(search.q) : search.q;
  const q = typeof raw === "string" ? raw.trim() : "";
  if (q) out.q = Array.from(q).slice(0, QUERY_MAX).join("");
  if (typeof search.lang === "string" && LANGUAGE.test(search.lang))
    out.lang = search.lang;
  return out;
}
