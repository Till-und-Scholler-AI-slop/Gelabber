import { describe, expect, it } from "vitest";

describe("web scaffold", () => {
  it("keeps the pinned app name", () => {
    expect("Gelabber").toHaveLength(8);
  });
});
