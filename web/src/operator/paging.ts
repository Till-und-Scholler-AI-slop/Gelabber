import { useState } from "react";

import { PAGE_SIZE } from "./parts.tsx";

/** One loaded page: how many rows it holds and the list's total. */
export type PageCount = { rows: number; total: number };

/**
 * Where to stand after a page came back: an action that removed the last
 * row of a later page leaves it empty, so step back to the last page that
 * still has rows. Otherwise stay.
 */
export function settledOffset(offset: number, page: PageCount): number {
  if (page.rows > 0 || offset === 0 || offset < page.total) return offset;
  return Math.max(0, Math.floor((page.total - 1) / PAGE_SIZE) * PAGE_SIZE);
}

/**
 * Paging starts over whenever the search or the filter (`key`) changes.
 * `settle` takes the current page (not a placeholder from another offset)
 * and moves off a page an action emptied.
 */
export function usePaging(key: string) {
  const [paging, setPaging] = useState({ key, offset: 0 });
  const offset = paging.key === key ? paging.offset : 0;
  const settle = (page: PageCount | undefined) => {
    if (!page) return;
    const next = settledOffset(offset, page);
    if (next !== offset) setPaging({ key, offset: next });
  };
  return [
    offset,
    (next: number) => setPaging({ key, offset: next }),
    settle,
  ] as const;
}
