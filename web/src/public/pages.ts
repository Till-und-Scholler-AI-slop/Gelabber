// What this instance published (issue #522, "/public-pages.json"): the web
// container writes the file at start. The sign-in pages link the legal
// pages from it, and the brand leads to the landing page while it is on.

import { useQuery } from "@tanstack/react-query";

import type { Locale } from "../i18n/locale.ts";
import { LEGAL_PAGES, otherLocale, type LegalPageId } from "./urls.ts";

/** A published legal page's path per language. */
export type LegalPageUrls = Partial<Record<Locale, string>>;

export type PublicPages = {
  readonly landing: boolean;
  readonly legal: Partial<Record<LegalPageId, LegalPageUrls>>;
};

/** A missing, broken or unknown file: nothing is published. */
export const NO_PUBLIC_PAGES: PublicPages = { landing: false, legal: {} };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

// Only a path on this origin: the links are plain anchors.
function isLocalPath(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.startsWith("/") &&
    !value.startsWith("//") &&
    !value.includes("\\")
  );
}

/** The file's contents, read as version 1; anything else publishes nothing. */
export function parsePublicPages(json: unknown): PublicPages {
  if (!isRecord(json) || json.version !== 1) return NO_PUBLIC_PAGES;
  const legal: Partial<Record<LegalPageId, LegalPageUrls>> = {};
  const pages = isRecord(json.legal) ? json.legal : {};
  for (const id of LEGAL_PAGES) {
    const entry = pages[id];
    if (!isRecord(entry)) continue;
    const urls: LegalPageUrls = {};
    if (isLocalPath(entry.de)) urls.de = entry.de;
    if (isLocalPath(entry.en)) urls.en = entry.en;
    if (urls.de || urls.en) legal[id] = urls;
  }
  return { landing: json.landing === true, legal };
}

/** Not under /api: nginx serves the file next to the pages. */
export async function fetchPublicPages(
  signal?: AbortSignal,
): Promise<PublicPages> {
  try {
    const response = await fetch("/public-pages.json", {
      signal,
      headers: { Accept: "application/json" },
    });
    if (!response.ok) return NO_PUBLIC_PAGES;
    return parsePublicPages(await response.json());
  } catch (error) {
    if (signal?.aborted) throw error;
    return NO_PUBLIC_PAGES;
  }
}

/**
 * Fixed per deployment, so it is asked once. Until it answers, nothing is
 * published, which is also the self-hosted default.
 */
export function usePublicPages(): PublicPages {
  const { data } = useQuery({
    queryKey: ["instance", "public-pages"],
    queryFn: ({ signal }) => fetchPublicPages(signal),
    staleTime: Infinity,
    retry: false,
  });
  return data ?? NO_PUBLIC_PAGES;
}

/** The page's path in this language, else in the other one. */
export function legalPageUrl(
  urls: LegalPageUrls,
  locale: Locale,
): string | undefined {
  return urls[locale] ?? urls[otherLocale(locale)];
}
