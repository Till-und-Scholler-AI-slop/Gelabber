// How a room's pictures are laid out: one spotlight, a filmstrip of the
// other shares, and cameras in their owners' places among the participants.

export type StageKind = "live" | "screen" | "camera";

/** Live first, then screens: a share takes the spotlight unless someone
 * pinned another picture there, a camera included. The filmstrip holds the
 * shares that are not in it; cameras stay with their owners. */
export function arrangeStage<T extends { id: string; kind: StageKind }>(
  shares: readonly T[],
  cameras: readonly T[],
  pinned: string | null,
): { spotlight: T | null; filmstrip: T[] } {
  const spotlight =
    [...shares, ...cameras].find((tile) => tile.id === pinned) ??
    shares[0] ??
    null;
  return {
    spotlight,
    filmstrip: shares.filter((tile) => tile !== spotlight),
  };
}

/** How a tile gets the whole screen: the element itself where the page may
 * go fullscreen, the video's own player on an iPhone, the large view
 * otherwise. */
export function fullscreenRoute(
  fullscreenEnabled: boolean | undefined,
  video: { webkitEnterFullscreen?: unknown } | null,
): "element" | "video" | "viewer" {
  if (fullscreenEnabled) return "element";
  if (typeof video?.webkitEnterFullscreen === "function") return "video";
  return "viewer";
}

/** No picture yet: nothing for a moment, then a spinner, then the news
 * that none arrives. */
export const FRAME_LOADING_MS = 300;
export const FRAME_STALLED_MS = 8_000;
/** Overlaid controls hide after this long without a pointer or key. */
export const CONTROLS_IDLE_MS = 3_000;
/** A press this long on a participant opens their menu. */
export const LONG_PRESS_MS = 500;
/** A finger that moves this far scrolls instead of pressing. */
const LONG_PRESS_SLOP = 10;

/** Press-and-hold on a touch screen, where no contextmenu event can be
 * relied on (iOS Safari fires none). Mouse presses are left alone. */
export function createLongPress(
  onLongPress: () => void,
  timers: {
    set: (run: () => void, ms: number) => unknown;
    clear: (handle: unknown) => void;
  } = {
    set: (run, ms) => setTimeout(run, ms),
    clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
  },
) {
  let timer: unknown = null;
  let origin: { x: number; y: number } | null = null;
  let fired = false;
  const cancel = () => {
    if (timer !== null) timers.clear(timer);
    timer = null;
    origin = null;
  };
  return {
    down(event: { pointerType: string; clientX: number; clientY: number }) {
      cancel();
      fired = false;
      if (event.pointerType === "mouse") return;
      origin = { x: event.clientX, y: event.clientY };
      timer = timers.set(() => {
        timer = null;
        fired = true;
        onLongPress();
      }, LONG_PRESS_MS);
    },
    move(event: { clientX: number; clientY: number }) {
      if (
        origin &&
        Math.hypot(event.clientX - origin.x, event.clientY - origin.y) >
          LONG_PRESS_SLOP
      )
        cancel();
    },
    up: cancel,
    /** Whether the click that ends this press already did its work. */
    consumeClick(): boolean {
      const was = fired;
      fired = false;
      return was;
    },
  };
}
