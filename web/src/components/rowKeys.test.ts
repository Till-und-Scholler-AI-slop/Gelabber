import { describe, expect, it } from "vitest";

import { rowStep } from "./rowKeys.ts";

describe("sidebar row keys", () => {
  it("moves right and left and stops at the ends", () => {
    expect(rowStep("ArrowRight", 0, 3)).toBe(1);
    expect(rowStep("ArrowRight", 2, 3)).toBe(2);
    expect(rowStep("ArrowLeft", 2, 3)).toBe(1);
    expect(rowStep("ArrowLeft", 0, 3)).toBe(0);
  });

  it("jumps to the first and last item", () => {
    expect(rowStep("Home", 2, 3)).toBe(0);
    expect(rowStep("End", 0, 3)).toBe(2);
  });

  it("leaves other keys and focus outside the row alone", () => {
    expect(rowStep("ArrowDown", 0, 3)).toBeNull();
    expect(rowStep("Tab", 0, 3)).toBeNull();
    expect(rowStep("ArrowRight", -1, 3)).toBeNull();
    expect(rowStep("ArrowRight", 0, 0)).toBeNull();
  });
});
