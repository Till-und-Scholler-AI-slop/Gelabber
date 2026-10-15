import type { KeyboardEvent } from "react";
import { describe, expect, it, vi } from "vitest";

import { moveWithinRow, neighbourRow, rowStep } from "./rowKeys.ts";

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

// The web tests run without a DOM; a row is its items and their focus().
function row(count: number) {
  const items = Array.from({ length: count }, () => ({ focus: vi.fn() }));
  const element = { querySelectorAll: () => items };
  const press = (key: string, target: unknown) => {
    const event = {
      key,
      target,
      currentTarget: element,
      preventDefault: vi.fn(),
    };
    moveWithinRow(event as unknown as KeyboardEvent<HTMLElement>);
    return event.preventDefault;
  };
  return { items, press };
}

describe("moving within a row", () => {
  it("focuses the next, previous, first and last item", () => {
    const { items, press } = row(3);
    expect(press("ArrowRight", items[0])).toHaveBeenCalledOnce();
    expect(items[1]!.focus).toHaveBeenCalledOnce();
    press("ArrowLeft", items[2]);
    expect(items[1]!.focus).toHaveBeenCalledTimes(2);
    press("End", items[0]);
    expect(items[2]!.focus).toHaveBeenCalledOnce();
    press("Home", items[2]);
    expect(items[0]!.focus).toHaveBeenCalledOnce();
  });

  it("leaves keys that move nothing to the browser", () => {
    const { items, press } = row(3);
    expect(press("ArrowDown", items[0])).not.toHaveBeenCalled();
    expect(press("Tab", items[1])).not.toHaveBeenCalled();
    expect(press("ArrowRight", items[2])).not.toHaveBeenCalled();
    expect(press("Home", items[0])).not.toHaveBeenCalled();
    for (const item of items) expect(item.focus).not.toHaveBeenCalled();
  });

  it("keeps Home and End for scrolling in a row without actions", () => {
    const { items, press } = row(1);
    expect(press("Home", items[0])).not.toHaveBeenCalled();
    expect(press("End", items[0])).not.toHaveBeenCalled();
  });

  it("ignores keys from outside the row's items", () => {
    const { items, press } = row(3);
    expect(press("ArrowRight", { focus: vi.fn() })).not.toHaveBeenCalled();
    for (const item of items) expect(item.focus).not.toHaveBeenCalled();
  });
});

describe("focus after a removed row", () => {
  it("takes the row now in its place, else the one before", () => {
    expect(neighbourRow([1, 3, 4, 6], 4)).toBe(4);
    expect(neighbourRow([6, 1, 3], 4)).toBe(6);
    expect(neighbourRow([1, 3], 4)).toBe(3);
    expect(neighbourRow([], 4)).toBeNull();
  });
});
