export type ScrollPosition = {
  top: number;
  height: number;
  viewport: number;
};

export const endDistance = (position: ScrollPosition) =>
  position.height - position.top - position.viewport;

/**
 * Whether the list is pinned to its end after a scroll from `previous` to
 * `current`. A scroll in which the height or the visible height changed may
 * be the layout's alone (a clamp, a row measured above the reader), and those
 * are no new reading intent: the old value stays, so a picture loading under
 * a reader at the end keeps them there. Two exceptions:
 *
 * - `byReader`: the reader moved the list themselves (the scrollbar), so the
 *   move counts whatever else changed;
 * - moves the layout cannot make. A clamp leaves the list exactly at its
 *   end; a row measured above the reader moves the list and its end alike,
 *   and only while the list is not pinned (useMessageScroll). So a move down
 *   to the end itself is at the end, however it got there (GEL-456), and a
 *   move up that ends away from the end leaves it (GEL-461).
 */
export function scrollEndIntent(
  pinned: boolean,
  previous: ScrollPosition | null,
  current: ScrollPosition,
  byReader = false,
): boolean {
  if (!previous) return pinned;
  const moved = current.top - previous.top;
  if (
    !byReader &&
    (previous.height !== current.height ||
      previous.viewport !== current.viewport)
  )
    return moved > 0 && endDistance(previous) > 1 && endDistance(current) <= 1
      ? true
      : moved < 0 && endDistance(current) > 1
        ? false
        : pinned;
  if (moved < 0) return false;
  if (moved > 0 && endDistance(current) < 96) return true;
  return pinned;
}
