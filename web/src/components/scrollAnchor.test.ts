import { describe, expect, it } from "vitest";

import { anchoredScrollTop } from "./scrollAnchor.ts";

/** Rows of fixed heights stacked from 0, like the virtualizer lays them out. */
function layout(rows: { id: string; height: number }[]) {
  const starts: number[] = [];
  rows.reduce((top, row, index) => {
    starts[index] = top;
    return top + row.height;
  }, 0);
  return (index: number) => starts[index];
}

const rows = (...spec: [string, number][]) =>
  spec.map(([id, height]) => ({ id, height }));

// The arithmetic only. The #201 jump came from where the start offsets were
// read (clamped, before the new rows were measured) and from when they were
// applied; this cannot see either. That behaviour is checked in a browser by
// `scripts/smoke-chat-scroll.mjs`.
describe("scroll anchor", () => {
  it("keeps the top row in place when older rows load above it", () => {
    const before = rows(["m105", 80], ["m106", 260], ["m107", 80]);
    // Scrolled so m106 starts 30 px above the viewport top.
    const scrollTop = layout(before)(1) + 30;
    const anchor = { id: "m106", offset: layout(before)(1) - scrollTop };

    const after = rows(
      ["m40", 80],
      ["m41", 300],
      ...before.map((r) => [r.id, r.height] as [string, number]),
    );
    const top = anchoredScrollTop(after, anchor, layout(after));
    expect(top).toBe(80 + 300 + 80 + 30);
    expect(layout(after)(3) - top!).toBe(anchor.offset);
  });

  it("keeps the top row in place when a row above it is removed", () => {
    // Before: a, b, c at 0, 100, 200; c 10 px below the top edge.
    const anchor = { id: "c", offset: -10 };
    const after = rows(["b", 100], ["c", 100]);
    expect(anchoredScrollTop(after, anchor, layout(after))).toBe(110);
  });

  it("leaves the position alone when the anchor row is gone or unmeasured", () => {
    const list = rows(["a", 100]);
    expect(anchoredScrollTop(list, { id: "z", offset: 0 }, layout(list))).toBe(
      undefined,
    );
    expect(anchoredScrollTop(list, null, layout(list))).toBe(undefined);
    expect(
      anchoredScrollTop(list, { id: "a", offset: 0 }, () => undefined),
    ).toBe(undefined);
  });
});
