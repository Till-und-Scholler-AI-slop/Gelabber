import { describe, expect, it } from "vitest";

import {
  ALT_TEXT_MAX,
  altTextLength,
  altTextOf,
  setAltText,
  withoutControls,
} from "./altText.ts";

describe("alt texts of files not sent yet (#575)", () => {
  it("belong to the very file, trimmed, and blank removes them", () => {
    const a = new File(["a"], "a.png", { type: "image/png" });
    const b = new File(["a"], "a.png", { type: "image/png" });
    setAltText(a, "  Eine Katze \n");
    expect(altTextOf(a)).toBe("Eine Katze");
    expect(altTextOf(b)).toBeUndefined();
    setAltText(a, "   ");
    expect(altTextOf(a)).toBeUndefined();
  });

  it("drops control characters but keeps tabs and line breaks", () => {
    expect(withoutControls("a\u000bb\u000cc\u0000d\u007f\te\nf")).toBe(
      "abcd\te\nf",
    );
  });

  it("counts characters as the server does", () => {
    expect(altTextLength("😀".repeat(ALT_TEXT_MAX))).toBe(ALT_TEXT_MAX);
    expect(altTextLength(" ä ")).toBe(1);
  });
});
