import { describe, expect, it } from "vitest";

import { nameList, roomSummary } from "./roomSummary.ts";

describe("room summary", () => {
  it("invites the first one in, where they may join", () => {
    expect(roomSummary([], { here: false, canJoin: true })).toBe(
      "Noch niemand da – komm als Erste:r dazu.",
    );
    expect(roomSummary([], { here: false, canJoin: false })).toBe(
      "Gerade ist niemand da.",
    );
  });

  it("names who is there", () => {
    const outside = { here: false, canJoin: true };
    expect(roomSummary(["Mia"], outside)).toBe("Mia ist da.");
    expect(roomSummary(["Mia", "Lena"], outside)).toBe("Mia und Lena sind da.");
    expect(roomSummary(["Mia", "Lena", "Kim", "Jo"], outside)).toBe(
      "Mia, Lena und 2 weitere sind da.",
    );
  });

  it("speaks to someone inside", () => {
    expect(roomSummary(["Mia"], { here: true, canJoin: true })).toBe(
      "Du bist hier mit Mia.",
    );
    expect(roomSummary([], { here: true, canJoin: true })).toContain(
      "Du bist da.",
    );
  });

  it("lists three by name", () => {
    expect(nameList(["Mia", "Lena", "Kim"])).toBe("Mia, Lena und Kim");
  });
});
