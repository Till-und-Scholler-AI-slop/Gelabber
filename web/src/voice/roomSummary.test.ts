import { describe, expect, it } from "vitest";

import {
  liveSentence,
  nameList,
  roomSummary,
  stageFacts,
} from "./roomSummary.ts";

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

describe("stage facts", () => {
  const facts = (
    cameras: number,
    live: string[] = [],
    selfLive = false,
    people = 3,
  ) => stageFacts({ people, cameras, live, selfLive });

  it("counts the people and the cameras", () => {
    expect(facts(2)).toBe("3 im Kanal · 2 Kameras");
    expect(facts(1, [], false, 2)).toBe("2 im Kanal · 1 Kamera");
    expect(facts(0)).toBe("3 im Kanal");
  });

  it("says who is live instead of counting cameras", () => {
    expect(facts(2, ["Jonas"])).toBe("3 im Kanal · Jonas ist live");
    expect(facts(0, ["Jonas", "Mia"])).toBe(
      "3 im Kanal · Jonas und Mia sind live",
    );
    expect(facts(1, [], true)).toBe("3 im Kanal · Du bist live");
    expect(facts(1, ["Mia"], true)).toBe("3 im Kanal · Du und Mia seid live");
  });
});
