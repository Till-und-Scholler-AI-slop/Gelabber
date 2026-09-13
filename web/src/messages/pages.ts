// Pure helpers for the infinite-query cache: flatten oldest→newest, pick
// the cursor for the next older page, overlay in-flight optimistic rows.

import type { Message, MessagePage } from "./types.ts";

/** `pages[0]` is the newest batch; later pages are older. */
export function flattenPages(pages: MessagePage[]): Message[] {
  const out: Message[] = [];
  for (let i = pages.length - 1; i >= 0; i--) {
    const page = pages[i];
    if (page) out.push(...page.messages);
  }
  return out;
}

/** Id of the oldest row in this page, when the server says more exist. */
export function olderCursor(page: MessagePage): string | undefined {
  if (!page.has_more || page.messages.length === 0) return undefined;
  return page.messages[0]?.id;
}

/**
 * History (oldest → newest) plus pending sends that are not yet in a page.
 * Pending ids are `tmp:…`; they sit at the bottom so the row appears at once.
 */
export function visibleMessages(
  pages: MessagePage[],
  pending: Message[],
): Message[] {
  if (pending.length === 0) return flattenPages(pages);
  const seen = new Set(flattenPages(pages).map((m) => m.id));
  const extra = pending.filter((m) => !seen.has(m.id));
  return extra.length === 0
    ? flattenPages(pages)
    : [...flattenPages(pages), ...extra];
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
