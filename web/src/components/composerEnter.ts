// What Enter does in the composer. All of its rules live in `enterSends`
// (GEL-143 the open code block; keep further ones, such as the touch
// keyboard's, in the same function).

const FENCE = "```";

/**
 * Whether `caret` is inside a ``` code block that is not closed before it,
 * reading the text as the message markdown does (messages/markdown.ts): a
 * backslash outside a block escapes the next character.
 */
export function inOpenFence(text: string, caret: number): boolean {
  let open = false;
  let i = 0;
  while (i < caret) {
    if (!open && text[i] === "\\") {
      i += 2;
      continue;
    }
    if (i + FENCE.length <= caret && text.startsWith(FENCE, i)) {
      open = !open;
      i += FENCE.length;
      continue;
    }
    i += 1;
  }
  return open;
}

/**
 * Whether a press of Enter sends the draft. Otherwise the field keeps the
 * key: with Shift or inside an open code block it breaks the line, and the
 * Enter that ends an input method's composition only ends that.
 */
export function enterSends(
  key: { shiftKey: boolean; isComposing: boolean },
  text: string,
  caret: number,
): boolean {
  if (key.shiftKey || key.isComposing) return false;
  return !inOpenFence(text, caret);
}
