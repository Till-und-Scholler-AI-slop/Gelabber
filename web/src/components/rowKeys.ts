// A sidebar row with actions is one Tab stop; the arrow keys move between
// its link and its buttons (roving focus, like a toolbar).

import type { KeyboardEvent } from "react";

/** Index to focus after `key`, or `null` when the key is not ours. */
export function rowStep(
  key: string,
  index: number,
  count: number,
): number | null {
  if (index < 0 || count === 0) return null;
  switch (key) {
    case "ArrowRight":
      return Math.min(index + 1, count - 1);
    case "ArrowLeft":
      return Math.max(index - 1, 0);
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** Marks the elements the arrow keys move between, in DOM order. */
export const ROW_ITEM = { "data-row-item": "" } as const;

export function moveWithinRow(event: KeyboardEvent<HTMLElement>): void {
  const items = [
    ...event.currentTarget.querySelectorAll<HTMLElement>("[data-row-item]"),
  ];
  const active = document.activeElement;
  const next = rowStep(
    event.key,
    items.findIndex((item) => item === active),
    items.length,
  );
  if (next === null) return;
  event.preventDefault();
  items[next]?.focus();
}
