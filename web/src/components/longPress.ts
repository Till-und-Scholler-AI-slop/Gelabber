// A long press on the app's own chrome opens no browser menu on a touch
// screen. iOS takes that from -webkit-touch-callout in index.css; Android
// only from a cancelled contextmenu event. Both name the same areas.

const CHROME =
  ".workspace-drawer, .workspace-mobile-topbar, .lr-channel-header, .app-header, .voice-session-dock";
// Nested in chrome, but with text to copy or a field to paste into.
const CONTENT = ".gel-modal, input, textarea";

/** Whether a contextmenu event is a long press on chrome, not on content. */
export function isChromeLongPress(
  event: { target: EventTarget | null; pointerType?: string },
  touchScreen: boolean,
): boolean {
  // Where the browser reports the pointer, a right click keeps its menu.
  if (!touchScreen || event.pointerType === "mouse") return false;
  const target = event.target as Partial<Element> | null;
  const owner = target?.closest?.(`${CONTENT}, ${CHROME}`);
  return owner?.matches(CHROME) ?? false;
}

/**
 * After a long press opened something, the finger's lift may still click
 * whatever is under it now, such as a sheet's backdrop. Swallows that one
 * click; a browser that sends none drops the guard at the next touch.
 */
export function swallowNextClick(): void {
  const swallow = (event: Event) => {
    event.stopPropagation();
    event.preventDefault();
    stop();
  };
  const stop = () => {
    window.removeEventListener("click", swallow, true);
    window.removeEventListener("pointerdown", stop, true);
  };
  window.addEventListener("click", swallow, true);
  window.addEventListener("pointerdown", stop, true);
}

/** How long a finger rests on a message before its actions open. */
export const LONG_PRESS_MS = 450;
/** How far it may drift meanwhile (px): further is a scroll. */
const LONG_PRESS_SLOP = 10;

type Point = { x: number; y: number };

/**
 * A long press from pointer events, which iOS Safari also sends (it has no
 * contextmenu event for a held finger). Touch and pen only: a mouse has its
 * right click.
 */
export function longPressGesture(onLongPress: () => void) {
  let start: Point | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const stop = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    start = null;
  };
  return {
    down(event: { pointerType: string; isPrimary: boolean } & Point) {
      stop();
      if (event.pointerType === "mouse" || !event.isPrimary) return;
      start = { x: event.x, y: event.y };
      timer = setTimeout(() => {
        timer = null;
        start = null;
        onLongPress();
      }, LONG_PRESS_MS);
    },
    move(event: Point) {
      if (
        start &&
        Math.hypot(event.x - start.x, event.y - start.y) > LONG_PRESS_SLOP
      )
        stop();
    },
    /** Lifted, cancelled (the browser took over to scroll) or left. */
    end: stop,
  };
}
