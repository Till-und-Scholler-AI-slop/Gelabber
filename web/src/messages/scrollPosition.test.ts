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

  it("unpins when scrolling up and repins only after scrolling down near end", () => {
    expect(scrollEndIntent(true, at(2160), at(2140))).toBe(false);
    expect(scrollEndIntent(false, at(1700), at(1900))).toBe(false);
    expect(scrollEndIntent(false, at(2000), at(2100))).toBe(true);
    expect(scrollEndIntent(false, at(2100), at(2100))).toBe(false);
    expect(scrollEndIntent(false, null, at(2160))).toBe(false);
  });
});
