// The server's GIF relay (`/api/gifs`). The browser never talks to the
// provider: search goes through the API and so do the GIF bytes.

import { ApiError, api } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import type { Gif } from "../messages/types.ts";

export type GifStatus = { enabled: boolean; provider: string | null };

export type GifPage = { gifs: Gif[]; next_page: number | null };

/** What the composer posts: a picker result and the search that found it. */
export type GifPick = { slug: string; query: string; gif: Gif };

export const gifStatus = (signal?: AbortSignal) =>
  api<GifStatus>("/gifs", { signal });

export function gifPage(
  query: string,
  page: number,
  signal?: AbortSignal,
): Promise<GifPage> {
  const search = new URLSearchParams();
  const q = query.trim();
  if (q) search.set("q", q);
  if (page > 1) search.set("page", String(page));
  const encoded = search.toString();
  return api<GifPage>(
    `/gifs/${q ? "search" : "trending"}${encoded ? `?${encoded}` : ""}`,
    { signal },
  );
}

export function gifMediaUrl(url: string): string {
  return `/api/gifs/media?${new URLSearchParams({ url }).toString()}`;
}

/** The box a GIF gets in a message: at most 320 × 240, never upscaled. A
 *  fixed box keeps the row's height before the picture arrives. */
export function gifBox(
  gif: Pick<Gif, "width" | "height">,
  maxWidth = 320,
  maxHeight = 240,
): { width: number; height: number } {
  const scale = Math.min(1, maxWidth / gif.width, maxHeight / gif.height);
  return {
    width: Math.max(1, Math.round(gif.width * scale)),
    height: Math.max(1, Math.round(gif.height * scale)),
  };
}

export function gifLabel(gif: Pick<Gif, "title">): string {
  return gif.title ? `GIF: ${gif.title}` : "GIF";
}

/** The picker's status line. A full budget says when to try again instead
 *  of a generic failure. */
export function gifFeedStatus({
  pending,
  error,
  count,
  query,
}: {
  pending: boolean;
  error: unknown;
  count: number;
  query: string;
}): string {
  if (pending) return "GIFs werden geladen …";
  if (error) {
    if (error instanceof ApiError && error.code === "rate_limited")
      return error.retryAfter
        ? `Zu viele GIF-Anfragen. Bitte in ${error.retryAfter} s noch einmal versuchen.`
        : errorMessage("rate_limited");
    if (error instanceof ApiError && error.code === "upstream_unavailable")
      return errorMessage("upstream_unavailable");
    if (count === 0) return "GIFs konnten nicht geladen werden.";
    return "Weitere GIFs konnten nicht geladen werden.";
  }
  if (count === 0) return "Keine GIFs gefunden.";
  return query ? `${count} GIFs zu „${query}“` : `${count} angesagte GIFs`;
}
