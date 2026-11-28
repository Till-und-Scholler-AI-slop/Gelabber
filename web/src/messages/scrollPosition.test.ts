import { describe, expect, it } from "vitest";
import { endDistance, scrollEndIntent } from "./scrollPosition.ts";

const at = (top: number, height = 2600, viewport = 440) => ({
  top,
  height,
  viewport,
});

describe("message-list reading intent", () => {
  it("keeps history intent when a larger viewport clamps it to the end", () => {
    const before = at(2100);
    const clamped = at(2100, 2600, 500);
    expect(endDistance(before)).toBe(60);
    expect(endDistance(clamped)).toBe(0);
    expect(scrollEndIntent(false, before, clamped)).toBe(false);
    expect(scrollEndIntent(false, clamped, at(2100))).toBe(false);
  });

  it("keeps history intent during row measurements and content deletion", () => {
    expect(scrollEndIntent(false, at(1900), at(1800, 2240))).toBe(false);
    expect(scrollEndIntent(false, at(1900), at(2140, 2800))).toBe(false);
  });

  it("retains the pinned intent across viewport and content changes", () => {
    expect(scrollEndIntent(true, at(2160), at(2100, 2600, 500))).toBe(true);
    expect(scrollEndIntent(true, at(2160), at(2300, 2800))).toBe(true);
  });

  it("keeps a reader at the end when rows grow under them without a move", () => {
    expect(scrollEndIntent(true, at(2160), at(2160, 2800))).toBe(true);
  });

  it("knows the list is at its end after a move down to it, though its height changed (GEL-456)", () => {
    // From the top of the history to the end while rows were measured: the
    // newest is on screen and must count as read.
    expect(scrollEndIntent(false, at(500), at(2360, 2800))).toBe(true);
    // Short of the end it stays in the history, as during measurements.
    expect(scrollEndIntent(false, at(500), at(2300, 2800))).toBe(false);
  });

  it("unpins a move up from the end that the layout cannot make, though the height changed (GEL-461)", () => {
    // Up from the end while a row grew, ending in the history: a clamp ends
    // at the end, and a row measured above a pinned list moves nothing.
    expect(scrollEndIntent(true, at(2160), at(1900, 2800))).toBe(false);
    // A clamp to the end keeps the reader there.
    expect(scrollEndIntent(true, at(2160), at(2100, 2600, 500))).toBe(true);
  });

  it("takes a move the reader made with the scrollbar as theirs, though the height changed", () => {
    // Down to near the end, not quite at it: pinned again, as with a wheel.
    expect(scrollEndIntent(false, at(1500), at(2300, 2800), true)).toBe(true);
    // Without the scrollbar that is a measurement, and stays in the history.
    expect(scrollEndIntent(false, at(1500), at(2300, 2800))).toBe(false);
  });

  it("unpins when scrolling up and repins only after scrolling down near end", () => {
    expect(scrollEndIntent(true, at(2160), at(2140))).toBe(false);
    expect(scrollEndIntent(false, at(1700), at(1900))).toBe(false);
    expect(scrollEndIntent(false, at(2000), at(2100))).toBe(true);
    expect(scrollEndIntent(false, at(2100), at(2100))).toBe(false);
    expect(scrollEndIntent(false, null, at(2160))).toBe(false);
  });
});
