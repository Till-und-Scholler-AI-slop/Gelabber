import { describe, expect, it } from "vitest";

import { builtinTheme } from "../theme/presets.ts";
import { toneOf } from "./pageTone.ts";

describe("the tone of a page background", () => {
  it("follows the background's luminance", () => {
    expect(toneOf("#ffffff", true)).toBe("light");
    expect(toneOf("#f4efe6", true)).toBe("light");
    expect(toneOf("#131314", false)).toBe("dark");
    expect(toneOf(" #0E1113 ", false)).toBe("dark");
  });

  it("matches the built-in light and dark themes", () => {
    expect(toneOf(builtinTheme("light").colors.background, true)).toBe("light");
    expect(toneOf(builtinTheme("dark").colors.background, false)).toBe("dark");
  });

  it("falls back to the page's mode before a theme is applied", () => {
    expect(toneOf("", true)).toBe("dark");
    expect(toneOf("", false)).toBe("light");
    expect(toneOf("rgb(0 0 0)", false)).toBe("light");
  });
});
