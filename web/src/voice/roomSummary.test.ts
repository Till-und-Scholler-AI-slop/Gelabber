import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { useLocale } from "../i18n/locale.ts";

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

  it('puts the rest in the dative after "mit"', () => {
    const others = ["Jonas", "Mira", ...Array.from({ length: 22 }, () => "X")];
    expect(roomSummary(others, { here: true })).toBe(
      "Du bist hier mit Jonas, Mira und 22 weiteren.",
    );
    expect(roomSummary(others, { here: false })).toBe(
      "Jonas, Mira und 22 weitere sind da.",
    );
    expect(nameList(others)).toBe("Jonas, Mira und 22 weitere");
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
  const facts = (cameras: number, lives = 0, people = 3) =>
    stageFacts({ people, cameras, lives });

  it("counts the people and the cameras", () => {
    expect(facts(2)).toBe("3 im Kanal · 2 Kameras");
    expect(facts(1, 0, 2)).toBe("2 im Kanal · 1 Kamera");
    expect(facts(0)).toBe("3 im Kanal");
  });

  it("counts the Lives instead of the cameras", () => {
    expect(facts(2, 1)).toBe("3 im Kanal · 1 Live");
    expect(facts(0, 2, 4)).toBe("4 im Kanal · 2 Live");
  });
});

describe("in English", () => {
  beforeEach(() => useLocale.setState({ locale: "en" }));
  afterEach(() => useLocale.setState({ locale: "de" }));

  it("lists names and says who is live", () => {
    expect(nameList(["Mia", "Lena", "Kim", "Jo"])).toBe("Mia, Lena and 2 more");
    expect(liveSentence(["You", "Mia"], true)).toBe("You and Mia are live");
    expect(liveSentence(["Mia"], false)).toBe("Mia is live");
    expect(roomSummary(["Mia", "Lena"], { here: false, live: ["Mia"] })).toBe(
      "Mia and Lena are here. Mia is live.",
    );
    expect(roomSummary([], { here: true })).toBe(
      "It’s just you in the channel.",
    );
  });

  it("counts on the stage", () => {
    const facts = (cameras: number, lives = 0) =>
      stageFacts({ people: 3, cameras, lives });
    expect(facts(1)).toBe("3 in the channel · 1 camera");
    expect(facts(2)).toBe("3 in the channel · 2 cameras");
    expect(facts(0, 1)).toBe("3 in the channel · 1 live");
    expect(roomSummary(["Mia", "Lena", "Kim", "Jo"], { here: true })).toBe(
      "You’re here with Mia, Lena and 2 more.",
    );
  });
});
