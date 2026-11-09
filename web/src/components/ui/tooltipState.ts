// The decisions of a tooltip (Tooltip.tsx) that do not need a page: where
// it is drawn, what Esc does to it, and that only one shows at a time.

/**
 * Where a control's tooltip is drawn, as Popover.tsx places itself: in the
 * modal dialog the control sits in (outside it the page is inert and
 * below the top layer), else in the element in fullscreen that holds the
 * control (outside it nothing shows), else on the page.
 */
export function tooltipHost<E>(
  anchor: { closest(selector: "dialog"): E | null },
  fullscreen: (E & { contains(node: unknown): boolean }) | null,
  page: E,
): E {
  const dialog = anchor.closest("dialog");
  if (dialog) return dialog;
  return fullscreen?.contains(anchor) ? fullscreen : page;
}

/**
 * Esc while a tooltip shows: it goes in any case (true). Shown for the
 * keyboard focus, putting it away is all the press does, as in the server
 * rail; under a resting pointer the press goes on to what it does anyway,
 * such as "als gelesen markieren".
 */
export function tooltipEscape(
  event: { key: string; defaultPrevented: boolean; preventDefault(): void },
  keyboard: boolean,
): boolean {
  if (event.key !== "Escape") return false;
  if (keyboard && !event.defaultPrevented) event.preventDefault();
  return true;
}

// The tooltip on the page, by the function that puts it away.
let current: (() => void) | null = null;

/** About to show: the one up elsewhere goes at once, so moving from one
 * control to the next never shows two. */
export function claimTooltip(hide: () => void): void {
  if (current && current !== hide) current();
  current = hide;
}

/** Gone: nothing to put away any more. */
export function releaseTooltip(hide: () => void): void {
  if (current === hide) current = null;
}
