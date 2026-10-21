import { describe, expect, it } from "vitest";

import { initials, tileInitials, tileTint } from "./initials.ts";

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

describe("server tile initials", () => {
  it("keeps two letters of a one-word name", () => {
    expect(tileInitials("Gaming")).toBe("Ga");
    expect(tileInitials("Gilde")).toBe("Gi");
    expect(tileInitials("spieleabend")).toBe("Sp");
    expect(tileInitials("X")).toBe("X");
  });

  it("uses the words' first letters otherwise", () => {
    expect(tileInitials("Freitag Runde")).toBe("FR");
    expect(tileInitials(" ")).toBe("?");
  });
});

describe("server tile tint", () => {
  it("is one of the eight tints and stable per id", () => {
    const tint = tileTint("0b6c3a2e-1d9f-4f55-9a3e-2b1c4d5e6f70");
    expect(tint).toMatch(/^var\(--lr-tile-[1-8]\)$/);
    expect(tileTint("0b6c3a2e-1d9f-4f55-9a3e-2b1c4d5e6f70")).toBe(tint);
  });

  it("spreads ids over the tints", () => {
    const tints = new Set(
      Array.from({ length: 64 }, (_, i) => tileTint(`server-${i}`)),
    );
    expect(tints.size).toBe(8);
  });
});
