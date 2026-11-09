import { describe, expect, it } from "vitest";

import { currentSection, scrolledToEnd, sectionInView } from "./sectionSpy.ts";

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

describe("section in view", () => {
  const page = { scrollTop: 0, clientHeight: 700, scrollHeight: 2000 };

  it("follows the headings without a jump", () => {
    expect(
      sectionInView({ ...page, headings: at(15, 400, 900), jump: null }),
    ).toEqual({ current: "s0", jump: null });
  });

  it("keeps a jump to a short last-but-one section at the end", () => {
    // Scrolled to the end, the headings alone would mark s2.
    const jump = { id: "s1", scrollTop: 1300 };
    const view = { ...page, scrollTop: 1300, headings: at(-900, 200, 400) };
    expect(sectionInView({ ...view, jump })).toEqual({ current: "s1", jump });
    expect(sectionInView({ ...view, jump: null }).current).toBe("s2");
  });

  it("keeps a jump on a page too short to scroll", () => {
    const jump = { id: "s1", scrollTop: 0 };
    const view = { headings: at(15, 300), scrollTop: 0, clientHeight: 700 };
    expect(sectionInView({ ...view, scrollHeight: 700, jump }).current).toBe(
      "s1",
    );
  });

  it("drops a jump once the reader scrolls on", () => {
    const jump = { id: "s2", scrollTop: 600 };
    const view = { ...page, headings: at(-600, -300, 15), jump };
    expect(sectionInView({ ...view, scrollTop: 601 }).jump).toBe(jump);
    expect(
      sectionInView({ ...view, scrollTop: 640, headings: at(-640, -340, -25) }),
    ).toEqual({ current: "s2", jump: null });
    expect(
      sectionInView({ ...view, scrollTop: 400, headings: at(-400, -100, 215) }),
    ).toEqual({ current: "s1", jump: null });
  });

  it("drops a jump when content loading above pushes its heading away", () => {
    // Invites arrived above Mitglieder: same offset, its heading 300px lower.
    const jump = { id: "s2", scrollTop: 600 };
    const view = { ...page, scrollTop: 600, jump };
    expect(sectionInView({ ...view, headings: at(-600, -300, 15) }).jump).toBe(
      jump,
    );
    expect(sectionInView({ ...view, headings: at(-600, 0, 315) })).toEqual({
      current: "s1",
      jump: null,
    });
  });

  it("drops a jump whose section is gone", () => {
    const jump = { id: "s5", scrollTop: 0 };
    expect(sectionInView({ ...page, headings: at(15, 400), jump }).jump).toBe(
      null,
    );
  });
});
