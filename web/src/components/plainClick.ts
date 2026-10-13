/** A primary click without modifiers: the app may take it over. Anything
 * else (middle click, Ctrl/Cmd for a new tab, Shift for a window, Alt to
 * save) stays the browser's. */
export function isPlainClick(event: {
  button: number;
  metaKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}): boolean {
  return (
    event.button === 0 &&
    !event.metaKey &&
    !event.ctrlKey &&
    !event.shiftKey &&
    !event.altKey
  );
}
