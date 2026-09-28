// Pure helpers for the infinite-query cache: flatten oldest→newest, pick
// the cursor for the next older page, overlay in-flight optimistic rows.

import type { Message, MessagePage } from "./types.ts";

export type MessageChange = {
  id: string;
  message: Message | null;
  /** A create may add a row absent from the snapshot within the loaded range. */
  created: boolean;
};

function chronological(
  a: Pick<Message, "id" | "created_at">,
  b: Pick<Message, "id" | "created_at">,
): number {
  return (
    compareTimestamp(a.created_at, b.created_at) || a.id.localeCompare(b.id)
  );
}

function compareTimestamp(a: string, b: string): number {
  const coarse = Date.parse(a) - Date.parse(b);
  if (coarse !== 0) return coarse;
  // Postgres/RFC3339 timestamps retain finer precision than Date.parse.
  const fraction = (value: string) =>
    (value.match(/\.(\d+)(?:Z|[+-]\d{2}:\d{2})$/)?.[1] ?? "").padEnd(9, "0");
  const left = fraction(a),
    right = fraction(b);
  return left === right ? 0 : left > right ? 1 : -1;
}

function newerMessage(current: Message, incoming: Message): Message {
  return compareTimestamp(
    current.edited_at ?? current.created_at,
    incoming.edited_at ?? incoming.created_at,
  ) > 0
    ? current
    : incoming;
}

/** Coalesce by immutable id, preserving tombstones and the newest edit. */
export function combineMessageChange(
  previous: MessageChange | undefined,
  next: MessageChange,
): MessageChange {
  if (previous?.message === null) return previous;
  return {
    ...next,
    created: next.created || previous?.created === true,
    message:
      previous?.message && next.message
        ? newerMessage(previous.message, next.message)
        : next.message,
  };
}

/** Overlay changes made while REST was reading, retaining pagination metadata. */
export function applyMessageChanges(
  pages: MessagePage[],
  changes: Iterable<MessageChange>,
): MessagePage[] {
  let result = pages;
  for (const change of changes) {
    let found = false;
    result = result.map((page) => ({
      ...page,
      messages: page.messages.flatMap((row) => {
        if (row.id !== change.id) return [row];
        found = true;
        return change.message ? [newerMessage(row, change.message)] : [];
      }),
    }));
    if (!found && change.created && change.message && result[0]) {
      const message = change.message;
      let index = result.findIndex((page) => {
        const start = snapshotStart(page);
        return start && chronological(message, start) >= 0;
      });
      // A delayed create outside the loaded range is fetched by paging. It
      // cannot extend a snapshot's boundary or claim the missing range exists.
      if (index < 0) {
        if (result.at(-1)?.has_more) continue;
        index = result.length - 1;
      }
      result = result.map((page, i) =>
        i === index
          ? {
              ...page,
              messages: [...page.messages, message].sort(chronological),
            }
          : page,
      );
    }
  }
  return result;
}

function snapshotStart(
  page: MessagePage,
): Pick<Message, "id" | "created_at"> | undefined {
  if (!page.older) return page.messages[0];
  const split = page.older.lastIndexOf("|");
  return {
    created_at: page.older.slice(0, split),
    id: page.older.slice(split + 1),
  };
}

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

/** Preserve the snapshot's boundary when live changes add or remove rows. */
export function olderCursor(page: MessagePage): string | undefined {
  if (!page.has_more) return undefined;
  if (page.older) return page.older;
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
