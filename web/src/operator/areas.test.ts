import { describe, expect, it } from "vitest";

import { OPERATOR_PAGES, SEARCH_MAX, validateOperatorSearch } from "./areas.ts";

describe("operator pages", () => {
  it("has Feedback between Meldungen and Auslastung", () => {
    expect(OPERATOR_PAGES).toEqual([
      "einladungen",
      "konten",
      "server",
      "meldungen",
      "feedback",
      "auslastung",
    ]);
  });
});

describe("operator search", () => {
  it("keeps a trimmed search of up to 100 characters", () => {
    expect(validateOperatorSearch({ q: "  Filmclub " })).toEqual({
      q: "Filmclub",
    });
    expect(validateOperatorSearch({ q: "x".repeat(150) }).q).toHaveLength(
      SEARCH_MAX,
    );
  });

  it("drops an empty or foreign value", () => {
    expect(validateOperatorSearch({})).toEqual({});
    expect(validateOperatorSearch({ q: "   " })).toEqual({});
    expect(validateOperatorSearch({ q: 7 })).toEqual({});
  });
});

describe("feedback entry", () => {
  const id = "0b9d3c55-8a51-4f0e-9f53-2d7c1e6a4b10";

  it("keeps a UUID, in lower case, beside a search", () => {
    expect(validateOperatorSearch({ entry: id })).toEqual({ entry: id });
    expect(validateOperatorSearch({ entry: id.toUpperCase(), q: "x" })).toEqual(
      { q: "x", entry: id },
    );
  });

  it("drops anything else", () => {
    expect(validateOperatorSearch({ entry: "42" })).toEqual({});
    expect(validateOperatorSearch({ entry: `${id}x` })).toEqual({});
    expect(validateOperatorSearch({ entry: 7 })).toEqual({});
  });
});
