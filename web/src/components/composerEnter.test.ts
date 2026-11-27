import { describe, expect, it } from "vitest";

import { enterSends, inOpenFence } from "./composerEnter.ts";

const plain = { shiftKey: false, isComposing: false };
/** `text` with the caret where `|` is. */
const at = (text: string): [string, number] => [
  text.replace("|", ""),
  text.indexOf("|"),
];

describe("Enter in the composer", () => {
  it("sends the draft", () => {
    expect(enterSends(plain, ...at("Hallo|"))).toBe(true);
    expect(enterSends(plain, ...at("```js\nlet a;\n```|"))).toBe(true);
  });

  it("breaks the line inside a code block that is still open", () => {
    expect(enterSends(plain, ...at("```|"))).toBe(false);
    expect(enterSends(plain, ...at("Schau:\n```js\nlet a = 1;|"))).toBe(false);
    // A second block opened after the first one closed.
    expect(enterSends(plain, ...at("```a``` und ```b|"))).toBe(false);
    // The caret before the fence that closes the block.
    expect(enterSends(plain, ...at("```\nlet a;|\n```"))).toBe(false);
  });

  it("reads the fences as the message markdown does", () => {
    // Escaped outside a block: no fence.
    expect(inOpenFence(...at("\\```|"))).toBe(false);
    // Inside a block a backslash escapes nothing: the block closes.
    expect(inOpenFence(...at("```a\\```|"))).toBe(false);
    // Two backticks of a fence typed so far open nothing yet.
    expect(inOpenFence(...at("``|`"))).toBe(false);
  });

  it("leaves Shift+Enter and the end of a composition to the field", () => {
    expect(enterSends({ ...plain, shiftKey: true }, ...at("Hallo|"))).toBe(
      false,
    );
    expect(enterSends({ ...plain, isComposing: true }, ...at("Hallo|"))).toBe(
      false,
    );
  });
});
