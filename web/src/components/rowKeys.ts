// A sidebar row with actions is one Tab stop; the arrow keys move between
// its link and its buttons. The stop roves like a toolbar's: it stays on
// the item last focused, so Tab away and Shift+Tab back lands there again.

import { useState, type FocusEvent, type KeyboardEvent } from "react";

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

function rowItems(row: HTMLElement): HTMLElement[] {
  return [...row.querySelectorAll<HTMLElement>("[data-row-item]")];
}

/**
 * Moves focus within the row. A key that would not move it (an arrow at
 * the end, Home or End in a row of one) is left to the browser, so Home
 * and End still scroll the list there.
 */
export function moveWithinRow(event: KeyboardEvent<HTMLElement>): void {
  const items = rowItems(event.currentTarget);
  const index = items.indexOf(event.target as HTMLElement);
  const next = rowStep(event.key, index, items.length);
  if (next === null || next === index) return;
  event.preventDefault();
  items[next]?.focus();
}

/**
 * The roving Tab stop of one row: spread `keys` on the row and give item
 * `i` `tabIndex(i)`. Off (every item in the normal Tab order) while the
 * row has no actions.
 */
export function useRovingRow(enabled: boolean) {
  const [current, setCurrent] = useState(0);
  return {
    keys: {
      onKeyDown: moveWithinRow,
      onFocus: (event: FocusEvent<HTMLElement>) => {
        const index = rowItems(event.currentTarget).indexOf(
          event.target as HTMLElement,
        );
        if (index >= 0) setCurrent(index);
      },
    },
    tabIndex: (index: number): number | undefined =>
      enabled ? (index === current ? 0 : -1) : undefined,
  };
}

/**
 * Where focus goes when the focused row at `index` was removed: the row
 * now in its place, else the nearest one before it. `rendered` are the
 * indices of the mounted rows that can take focus.
 */
export function neighbourRow(
  rendered: readonly number[],
  index: number,
): number | null {
  let before: number | null = null;
  let after: number | null = null;
  for (const candidate of rendered) {
    if (candidate >= index) {
      if (after === null || candidate < after) after = candidate;
    } else if (before === null || candidate > before) {
      before = candidate;
    }
  }
  return after ?? before;
}

/**
 * Focuses the Tab stop of the row nearest `index` among the mounted rows
 * (`[data-index]`) in `list`. Rows without items (headings, "Leer",
 * participants) are skipped.
 */
export function focusRowNear(list: HTMLElement, index: number): void {
  const stops = new Map<number, HTMLElement>();
  for (const row of list.querySelectorAll<HTMLElement>("[data-index]")) {
    const stop = row.querySelector<HTMLElement>(
      '[data-row-item]:not([tabindex="-1"])',
    );
    if (stop) stops.set(Number(row.dataset.index), stop);
  }
  const target = neighbourRow([...stops.keys()], index);
  if (target !== null) stops.get(target)?.focus();
}
