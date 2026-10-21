import { describe, expect, it } from "vitest";

import { readLinkToken } from "./mail.ts";

describe("readLinkToken", () => {
  const token = "ab".repeat(32);

  it("reads the token of a mailed link", () => {
    expect(readLinkToken(`#token=${token}`)).toBe(token);
    expect(readLinkToken(`#token=${token.toUpperCase()}`)).toBe(token);
    expect(readLinkToken(`#a=1&token=${token}&b=2`)).toBe(token);
  });

  it("ignores anything that is not a whole token", () => {
    expect(readLinkToken("")).toBe("");
    expect(readLinkToken("#token=abc")).toBe("");
    expect(readLinkToken(`#token=${token}0`)).toBe("");
    expect(readLinkToken(`#xtoken=${token}`)).toBe("");
  });
});
