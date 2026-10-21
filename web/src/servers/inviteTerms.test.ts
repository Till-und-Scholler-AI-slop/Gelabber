import { describe, expect, it } from "vitest";

import {
  DEFAULT_INVITE_TERMS,
  inviteInput,
  termsSummary,
} from "./inviteTerms.ts";

describe("invite terms", () => {
  it("defaults to a week, any number of people", () => {
    expect(inviteInput(DEFAULT_INVITE_TERMS)).toEqual({
      expires_in_hours: 168,
    });
    expect(termsSummary(DEFAULT_INVITE_TERMS)).toBe(
      "gültig 7 Tage · unbegrenzt nutzbar",
    );
  });

  it("leaves out what has no limit", () => {
    expect(inviteInput({ expiry: "never", uses: "unlimited" })).toEqual({});
    expect(inviteInput({ expiry: "1h", uses: "5" })).toEqual({
      expires_in_hours: 1,
      max_uses: 5,
    });
    expect(termsSummary({ expiry: "never", uses: "1" })).toBe(
      "unbegrenzt gültig · 1-mal nutzbar",
    );
    expect(termsSummary({ expiry: "24h", uses: "25" })).toBe(
      "gültig 24 Stunden · 25-mal nutzbar",
    );
  });
});
