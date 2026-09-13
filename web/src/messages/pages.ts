// Pure helpers for the infinite-query cache: flatten oldest→newest, pick
// the cursor for the next older page, overlay in-flight optimistic rows.

import type { Message, MessagePage } from "./types.ts";

/** Server `before`/`after`: time + id, so a hard delete of that row still pages. */
export function encodeCursor(
  message: Pick<Message, "id" | "created_at">,
): string {
  return `${message.created_at}|${message.id}`;
}

export function stampOlder(page: MessagePage): MessagePage {
  const oldest = page.messages[0];
  if (!oldest) return page;
  return { ...page, older: page.older ?? encodeCursor(oldest) };
}

/** `pages[0]` is the newest batch; later pages are older. Dedupes by id. */
export function flattenPages(pages: MessagePage[]): Message[] {
  const out: Message[] = [];
  const seen = new Set<string>();
  for (let i = pages.length - 1; i >= 0; i--) {
    const page = pages[i];
    if (!page) continue;
    for (const message of page.messages) {
      if (seen.has(message.id)) continue;
      seen.add(message.id);
      out.push(message);
    }
  }
  return out;
}

/** Composite cursor of the oldest row, or the stamp if the page was emptied. */
export function olderCursor(page: MessagePage): string | undefined {
  if (!page.has_more) return undefined;
  const oldest = page.messages[0];
  if (oldest) return encodeCursor(oldest);
  return page.older;
}

/**
 * History (oldest → newest) plus pending sends that are not yet in a page.
 * Dedupes by id so a confirmed overlay row and a GET row cannot both show.
 */
export function visibleMessages(
  pages: MessagePage[],
  pending: Message[],
): Message[] {
  const history = flattenPages(pages);
  if (pending.length === 0) return history;
  const seen = new Set(history.map((m) => m.id));
  const extra = pending.filter((m) => !seen.has(m.id));
  return extra.length === 0 ? history : [...history, ...extra];
}

/** Ids the overlay can drop — they already sit in a fetched page. */
export function confirmedPendingIds(
  pages: MessagePage[],
  pending: Message[],
): string[] {
  if (pending.length === 0) return [];
  const seen = new Set(flattenPages(pages).map((m) => m.id));
  return pending.filter((m) => seen.has(m.id)).map((m) => m.id);
}

/** Same author, same 5-minute window — compact row, no repeated avatar. */
export function isContinued(
  previous: Message | undefined,
  message: Message,
): boolean {
  if (!previous) return false;
  if (previous.author.id !== message.author.id) return false;
  const delta =
    Date.parse(message.created_at) - Date.parse(previous.created_at);
  return delta >= 0 && delta < 5 * 60 * 1000;
}
