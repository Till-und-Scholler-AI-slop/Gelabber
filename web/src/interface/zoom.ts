/**
 * The zoom the Textgröße puts on an element. `getBoundingClientRect` and the
 * window measure screen pixels; styles are set in the page's own CSS pixels,
 * which are this many times smaller.
 */
export function pageZoom(element: HTMLElement): number {
  const zoom =
    (element as { currentCSSZoom?: number }).currentCSSZoom ??
    element.getBoundingClientRect().width / (element.offsetWidth || 1);
  return zoom > 0 ? zoom : 1;
}
