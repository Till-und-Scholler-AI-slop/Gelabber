import { describe, expect, it } from "vitest";

import { focusRowAfterRemoval } from "./rowFocus.ts";

/** A list of row actions, and which of them has focus. */
function list(count: number, active: "body" | "elsewhere" | null = "body") {
  const body = { body: true };
  const focused: number[] = [];
  const actions = Array.from({ length: count }, (_, index) => ({
    focus: () => void focused.push(index),
  }));
  return {
    focused,
    list: {
      ownerDocument: {
        activeElement: active === "body" ? body : active && { other: true },
        body,
      },
      querySelectorAll: () => actions,
    },
  };
}

describe("focus after a row left its list", () => {
  it("goes to the action now in the row's place", () => {
    const rows = list(3);
    focusRowAfterRemoval(rows.list, 1);
    expect(rows.focused).toEqual([1]);
  });

  it("goes to the last action when the last row left", () => {
    const rows = list(2);
    focusRowAfterRemoval(rows.list, 2);
    expect(rows.focused).toEqual([1]);
  });

  it("also when focus is nowhere at all", () => {
    const rows = list(2, null);
    focusRowAfterRemoval(rows.list, 0);
    expect(rows.focused).toEqual([0]);
  });

  it("stays where the person moved it meanwhile", () => {
    const rows = list(3, "elsewhere");
    focusRowAfterRemoval(rows.list, 1);
    expect(rows.focused).toEqual([]);
  });
});
