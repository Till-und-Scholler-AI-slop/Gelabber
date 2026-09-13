import { describe, expect, it } from "vitest";

import { isOurTicket } from "./media.ts";

describe("media ticket shape", () => {
  it("accepts our 12-char alphabet and rejects product tokens", () => {
    expect(isOurTicket("abcdefghjkmn")).toBe(true);
    expect(isOurTicket("livekit_jwt_xxx")).toBe(false);
    expect(isOurTicket("SHORT")).toBe(false);
    expect(isOurTicket("abcdefghi0mn")).toBe(false);
  });
});
