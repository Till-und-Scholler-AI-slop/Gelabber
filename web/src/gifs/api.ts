// The server's GIF relay (`/api/gifs`). The browser never talks to the
// provider: search goes through the API and so do the GIF bytes.

import { ApiError, api } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { translate } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
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

/** "45 s", "2 Min." (rounded up): how long until the next try. */
function waitText(seconds: number): string {
  return seconds < 60
    ? translate(chat, "gif.waitSeconds", { seconds })
    : translate(chat, "gif.waitMinutes", { minutes: Math.ceil(seconds / 60) });
}

/** The picker's status line. A full budget (the user's own or the server's
 *  GIF quota) says when to try again instead of a generic failure. */
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
  if (pending) return translate(chat, "gif.loading");
  if (error) {
    if (error instanceof ApiError && error.code === "rate_limited")
      return error.retryAfter
        ? translate(chat, "gif.rateLimited", {
            seconds: String(error.retryAfter),
          })
        : errorMessage("rate_limited");
    if (error instanceof ApiError && error.code === "upstream_quota")
      return error.retryAfter
        ? translate(chat, "gif.quota", { wait: waitText(error.retryAfter) })
        : errorMessage("upstream_quota");
    if (error instanceof ApiError && error.code === "upstream_unavailable")
      return errorMessage("upstream_unavailable");
    if (count === 0) return translate(chat, "gif.failed");
    return translate(chat, "gif.moreFailed");
  }
  if (count === 0) return translate(chat, "gif.none");
  return query
    ? translate(chat, "gif.found", { count, query })
    : translate(chat, "gif.trending", { count });
}
