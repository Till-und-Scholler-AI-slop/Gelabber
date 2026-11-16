import { describe, expect, it } from "vitest";

import { blockedRuns } from "./runs.ts";

const by = (...ids: string[]) => ids.map((id) => ({ author: { id } }));

describe("blockedRuns", () => {
  it("folds nothing while nobody is blocked", () => {
    expect(blockedRuns(by("a", "b"), new Set())).toBeNull();
  });

  it("groups back-to-back messages of blocked people into one run", () => {
    const runs = blockedRuns(
      by("a", "x", "x", "y", "a", "x"),
      new Set(["x", "y"]),
    );
    expect(runs?.start).toEqual([-1, 1, 1, 1, -1, 5]);
    expect([...(runs?.length ?? [])]).toEqual([
      [1, 3],
      [5, 1],
    ]);
  });
});
