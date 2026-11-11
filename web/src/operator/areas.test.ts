import { describe, expect, it } from "vitest";

import { SEARCH_MAX, validateOperatorSearch } from "./areas.ts";

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
