// Which wheel, key and touch input belongs to the message list itself. Only
// that may move it out of following the newest message.

export type ScrollNode = {
  parentElement: ScrollNode | null;
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
};

/** Whether `target` is in the list's own DOM. A menu portaled out of a row
 * still passes its events up to the list through React. */
export function inList(target: unknown, list: ScrollNode): boolean {
  for (let node = target as ScrollNode | null; node; node = node.parentElement)
    if (node === list) return true;
  return false;
}

/** An element that scrolls its own overflow, like a text attachment. */
function ownScroller(node: ScrollNode): boolean {
  const overflow = getComputedStyle(node as unknown as Element).overflowY;
  return overflow === "auto" || overflow === "scroll";
}

/**
 * Whether a wheel or drag at `target` moves the list: it is in the list, and
 * no scroller between them can still move that way and takes it first.
 */
export function scrollsList(
  target: unknown,
  list: ScrollNode,
  direction: "up" | "down",
  scroller: (node: ScrollNode) => boolean = ownScroller,
): boolean {
  if (!inList(target, list)) return false;
  for (
    let node = target as ScrollNode | null;
    node && node !== list;
    node = node.parentElement
  ) {
    if (!scroller(node)) continue;
    const room =
      direction === "up"
        ? node.scrollTop > 0
        : node.scrollTop + node.clientHeight < node.scrollHeight - 1;
    if (room) return false;
  }
  return true;
}
