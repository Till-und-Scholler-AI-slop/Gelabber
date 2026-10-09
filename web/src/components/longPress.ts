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
