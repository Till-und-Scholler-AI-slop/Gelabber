export type PopoverPlacement =
  "bottom-start" | "bottom-end" | "top-start" | "top-end";

/** Phones and touch screens get a bottom sheet instead. */
export const POPOVER_SHEET_QUERY = "(max-width: 639px), (pointer: coarse)";

type Box = { top: number; right: number; bottom: number; left: number };

const GAP = 6;
const MARGIN = 8;

/**
 * Where an anchored popover goes: on the preferred side of its anchor when
 * it fits, flipped to the other side when only that fits, and always kept
 * `MARGIN` inside the viewport.
 *
 * `anchor` and `viewport` are screen pixels, as `getBoundingClientRect` and
 * `innerWidth` measure them; `size` and the result are the page's own CSS
 * pixels. They differ by `zoom`, the Textgröße set on the page.
 */
export function popoverPosition(
  screenAnchor: Box,
  size: { width: number; height: number },
  screenViewport: { width: number; height: number },
  placement: PopoverPlacement,
  zoom = 1,
): { top: number; left: number } {
  const anchor = {
    top: screenAnchor.top / zoom,
    right: screenAnchor.right / zoom,
    bottom: screenAnchor.bottom / zoom,
    left: screenAnchor.left / zoom,
  };
  const viewport = {
    width: screenViewport.width / zoom,
    height: screenViewport.height / zoom,
  };
  const below = anchor.bottom + GAP;
  const above = anchor.top - GAP - size.height;
  const fitsBelow = below + size.height <= viewport.height - MARGIN;
  const fitsAbove = above >= MARGIN;
  const preferBelow = placement.startsWith("bottom");
  const top =
    (preferBelow && (fitsBelow || !fitsAbove)) ||
    (!preferBelow && !fitsAbove && fitsBelow)
      ? below
      : above;
  const left = placement.endsWith("start")
    ? anchor.left
    : anchor.right - size.width;
  const clamp = (value: number, max: number) =>
    Math.max(MARGIN, Math.min(value, max - MARGIN));
  return {
    top: clamp(top, viewport.height - size.height),
    left: clamp(left, viewport.width - size.width),
  };
}

/**
 * Where an anchored popover grows from, as a CSS `transform-origin`: the
 * point on its edge facing the anchor, across from the anchor's middle.
 * Arguments as for `popoverPosition`, plus the position it returned.
 */
export function popoverOrigin(
  screenAnchor: Box,
  position: { top: number; left: number },
  size: { width: number; height: number },
  zoom = 1,
): string {
  const middle = (screenAnchor.left + screenAnchor.right) / 2 / zoom;
  const x = Math.max(0, Math.min(size.width, middle - position.left));
  const below = position.top >= screenAnchor.bottom / zoom;
  return `${Math.round(x)}px ${below ? 0 : size.height}px`;
}
