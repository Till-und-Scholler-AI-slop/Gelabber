import { describe, expect, it } from "vitest";

import { listingsPath } from "./api.ts";

describe("directory listings path", () => {
  it("sends only what is set", () => {
    expect(listingsPath({}, 0)).toBe("/directory/listings");
    expect(listingsPath({ q: "50% _off", language: "de" }, 30)).toBe(
      "/directory/listings?q=50%25+_off&language=de&offset=30",
    );
    expect(listingsPath({ language: "en" }, 0)).toBe(
      "/directory/listings?language=en",
    );
  });
});
