export type ScrollPosition = {
  top: number;
  height: number;
  viewport: number;
};

export const endDistance = (position: ScrollPosition) =>
  position.height - position.top - position.viewport;

/** Resize/measurement clamps are geometry changes, not new reading intent. */
export function scrollEndIntent(
  pinned: boolean,
  previous: ScrollPosition | null,
  current: ScrollPosition,
): boolean {
  if (
    !previous ||
    previous.height !== current.height ||
    previous.viewport !== current.viewport
  )
    return pinned;
  if (current.top < previous.top) return false;
  if (current.top > previous.top && endDistance(current) < 96) return true;
  return pinned;
}
