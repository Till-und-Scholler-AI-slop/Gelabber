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
 */
export function popoverPosition(
  anchor: Box,
  size: { width: number; height: number },
  viewport: { width: number; height: number },
  placement: PopoverPlacement,
): { top: number; left: number } {
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
