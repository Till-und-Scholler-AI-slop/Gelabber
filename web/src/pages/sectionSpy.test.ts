import { describe, expect, it } from "vitest";

import { currentSection, scrolledToEnd } from "./sectionSpy.ts";

const at = (...tops: number[]) =>
  tops.map((top, index) => ({ id: `s${index}`, top }));

describe("current section", () => {
  it("starts on the first section", () => {
    expect(currentSection(at(32, 400, 900), false)).toBe("s0");
    expect(currentSection(at(120, 400, 900), false)).toBe("s0");
  });

  it("moves on once a heading reaches the line under the anchor row", () => {
    expect(currentSection(at(-500, 15, 600), false)).toBe("s1");
    expect(currentSection(at(-500, 33, 600), false)).toBe("s0");
    expect(currentSection(at(-900, -400, -2), false)).toBe("s2");
  });

  it("takes the last section at the end of the page", () => {
    expect(currentSection(at(-900, -100, 300), true)).toBe("s2");
  });

  it("has none without sections", () => {
    expect(currentSection([], false)).toBeUndefined();
    expect(currentSection([], true)).toBeUndefined();
  });
});

describe("scrolled to the end", () => {
  it("needs a page that scrolls", () => {
    expect(scrolledToEnd(0, 800, 600)).toBe(false);
    expect(scrolledToEnd(0, 800, 2000)).toBe(false);
  });

  it("allows for rounding", () => {
    expect(scrolledToEnd(1199, 800, 2000)).toBe(true);
    expect(scrolledToEnd(1190, 800, 2000)).toBe(false);
  });
});
