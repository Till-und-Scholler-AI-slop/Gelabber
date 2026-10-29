import { describe, expect, it } from "vitest";

import { liveSentence, nameList, roomSummary } from "./roomSummary.ts";

describe("room summary", () => {
  it("says plainly that nobody is there", () => {
    expect(roomSummary([], { here: false })).toBe("Niemand im Kanal.");
  });

  it("names who is there", () => {
    const outside = { here: false };
    expect(roomSummary(["Mia"], outside)).toBe("Mia ist da.");
    expect(roomSummary(["Mia", "Lena"], outside)).toBe("Mia und Lena sind da.");
    expect(roomSummary(["Mia", "Lena", "Kim", "Jo"], outside)).toBe(
      "Mia, Lena und 2 weitere sind da.",
    );
  });

  it("says who is live outside, without asking to come along", () => {
    expect(
      roomSummary(["Mia", "Lena"], {
        here: false,
        live: ["Mia"],
      }),
    ).toBe("Mia und Lena sind da. Mia ist live.");
    expect(
      roomSummary(["Mia", "Lena"], {
        here: false,
        live: ["Mia", "Lena"],
      }),
    ).toBe("Mia und Lena sind da. Mia und Lena sind live.");
    expect(roomSummary(["Mia"], { here: true, live: ["Mia"] })).toBe(
      "Du bist hier mit Mia.",
    );
  });

  it("speaks to someone inside", () => {
    expect(roomSummary(["Mia"], { here: true })).toBe("Du bist hier mit Mia.");
    expect(roomSummary([], { here: true })).toBe("Nur du bist im Kanal.");
  });

  it("lists three by name", () => {
    expect(nameList(["Mia", "Lena", "Kim"])).toBe("Mia, Lena und Kim");
  });

  it("says who is live, the viewer first", () => {
    expect(liveSentence(["Mia"], false)).toBe("Mia ist live");
    expect(liveSentence(["Mia", "Lena"], false)).toBe("Mia und Lena sind live");
    expect(liveSentence(["Du"], true)).toBe("Du bist live");
    expect(liveSentence(["Du", "Mia"], true)).toBe("Du und Mia seid live");
  });
});
