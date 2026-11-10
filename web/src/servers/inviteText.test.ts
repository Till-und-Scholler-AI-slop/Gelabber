import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";

import { inviteExpiry, inviteUses } from "./inviteText.ts";

const now = Date.parse("2026-10-21T12:00:00Z");
const after = (ms: number) => new Date(now + ms).toISOString();
const HOUR = 3_600_000;

describe("invite text", () => {
  it("counts uses against the limit, or against none", () => {
    expect(inviteUses({ uses: 3, max_uses: null })).toBe("3/∞ benutzt");
    expect(inviteUses({ uses: 1, max_uses: 5 })).toBe("1/5 benutzt");
  });

  it("says when the link runs out in words", () => {
    expect(inviteExpiry(null, now)).toBe("läuft nie ab");
    expect(inviteExpiry(after(6 * 24 * HOUR), now)).toBe("läuft in 6 Tagen ab");
    expect(inviteExpiry(after(5 * HOUR), now)).toBe("läuft in 5 Stunden ab");
    expect(inviteExpiry(after(10 * 60_000), now)).toBe(
      "läuft in 10 Minuten ab",
    );
    expect(inviteExpiry(after(-HOUR), now)).toBe("abgelaufen");
  });
});

describe("invite text in English", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("counts uses and says when the link runs out", () => {
    setLocale("en", { persist: false });
    expect(inviteUses({ uses: 1, max_uses: 5 })).toBe("1/5 used");
    expect(inviteExpiry(null, now)).toBe("never expires");
    expect(inviteExpiry(after(6 * 24 * HOUR), now)).toBe("expires in 6 days");
    expect(inviteExpiry(after(5 * HOUR), now)).toBe("expires in 5 hours");
    expect(inviteExpiry(after(-HOUR), now)).toBe("expired");
  });
});
