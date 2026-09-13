import { describe, expect, it } from "vitest";

import { initials } from "./initials.ts";

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
