import { describe, expect, it } from "vitest";

import { initials, tileTone } from "./initials.ts";

describe("initials", () => {
  it("takes the first letter of up to two words", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("ada")).toBe("A");
    expect(initials("  Grace Brewster Hopper ")).toBe("GB");
    expect(initials("émile zola")).toBe("ÉZ");
  });

  it("falls back to a question mark", () => {
    expect(initials("")).toBe("?");
    expect(initials("   ")).toBe("?");
  });
});

describe("tile tone", () => {
  it("picks one of eight tints, the same one every time", () => {
    const ids = ["a", "b", "server-1", "7f2c9a1e-0000-4000-8000-000000000000"];
    for (const id of ids) {
      expect(tileTone(id)).toBeGreaterThanOrEqual(1);
      expect(tileTone(id)).toBeLessThanOrEqual(8);
      expect(tileTone(id)).toBe(tileTone(id));
    }
    expect(new Set(ids.map(tileTone)).size).toBeGreaterThan(1);
  });
});
