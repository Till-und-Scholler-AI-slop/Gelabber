import { describe, expect, it } from "vitest";

import { markMatches, searchTerms } from "./searchTerms.ts";

describe("search terms", () => {
  it("reads the words a websearch query looks for", () => {
    expect(searchTerms("Snacks")).toEqual(["snacks"]);
    expect(searchTerms('"Film oder" -Spiel or Chips')).toEqual([
      "film",
      "oder",
      "chips",
    ]);
    expect(searchTerms("  ")).toEqual([]);
  });

  it("marks whole words in any case", () => {
    expect(
      markMatches("Bringst du die Snacks mit? snacksalat", ["snacks"]),
    ).toEqual([
      { text: "Bringst du die ", match: false },
      { text: "Snacks", match: true },
      { text: " mit? snacksalat", match: false },
    ]);
    expect(markMatches("Äpfel und äpfel", ["äpfel"])).toEqual([
      { text: "Äpfel", match: true },
      { text: " und ", match: false },
      { text: "äpfel", match: true },
    ]);
    expect(markMatches("nichts", [])).toEqual([
      { text: "nichts", match: false },
    ]);
  });
});
