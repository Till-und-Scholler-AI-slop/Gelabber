// When a hover tooltip (the server rail's) shows. The first one waits a
// moment, so a pointer crossing the rail on its way elsewhere shows nothing;
// once one is up, moving to the next tile moves it there at once.

/** The wait on a first hover. */
export const TIP_DELAY_MS = 200;
/** How long a tooltip stays after the pointer left its tile, in case it
 * arrives at the next one, where it then simply moves. */
export const TIP_GRACE_MS = 80;
/** After a tooltip went, how long the next one still shows at once. */
export const TIP_WARM_MS = 400;

/** How long to wait before showing a tooltip. */
export function tipDelay({
  showing,
  warm,
  keyboard,
}: {
  /** One is up, or still lingering in its grace time. */
  showing: boolean;
  /** One went less than `TIP_WARM_MS` ago. */
  warm: boolean;
  /** Keyboard focus asked for it: whoever tabs there wants to know. */
  keyboard: boolean;
}): number {
  return showing || warm || keyboard ? 0 : TIP_DELAY_MS;
}
