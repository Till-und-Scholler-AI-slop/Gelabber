// The keys of the user panel's account popover (#570): the status field on
// top, the account's entries as a menu below it.

import type { KeyboardEvent } from "react";

/** The account menu's entries, in DOM order. */
function accountItems(root: HTMLElement): HTMLElement[] {
  return [
    ...root.querySelectorAll<HTMLElement>(
      '[role="menuitem"], [role="menuitemradio"]',
    ),
  ];
}

/**
 * The popover's keys: ↓ from the status field goes to the first entry; in
 * the menu ↑ and ↓ move (round the ends), Home and End jump, and ↑ on the
 * first entry goes back to the field. Esc is the Popover's: it closes and
 * hands the focus back to the avatar.
 */
export function moveInAccount(event: KeyboardEvent<HTMLElement>): void {
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
  const root = event.currentTarget;
  const items = accountItems(root);
  if (items.length === 0) return;
  const target = event.target as HTMLElement;
  const field = root.querySelector<HTMLInputElement>("#user-status-text");
  const index = items.indexOf(target);
  let next: HTMLElement | null | undefined;
  if (index < 0) {
    if (target !== field || event.key !== "ArrowDown") return;
    next = items[0];
  } else {
    const step = accountStep(event.key, index, items.length);
    if (step === null) return;
    next = step < 0 ? field : items[step];
  }
  if (!next) return;
  event.preventDefault();
  next.focus();
}

/** Entry to focus after `key` on entry `index` of `count`: -1 is the status
 * field above the menu, `null` a key that is not the menu's. */
export function accountStep(
  key: string,
  index: number,
  count: number,
): number | null {
  switch (key) {
    case "ArrowDown":
      return (index + 1) % count;
    case "ArrowUp":
      return index === 0 ? -1 : index - 1;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}
