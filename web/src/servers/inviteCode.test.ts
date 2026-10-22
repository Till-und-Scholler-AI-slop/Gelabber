import { describe, expect, it } from "vitest";

import { inviteExpiry, inviteInputMessage, parseInvite } from "./inviteCode.ts";

const origin = "https://gelabber.example";
const code = "k7xm2pqrstuv";

describe("invite input", () => {
  it("takes a full link, a link without scheme, a path or the code", () => {
    expect(
      parseInvite(`https://gelabber.example/invite/${code}`, origin),
    ).toEqual({ code });
    expect(parseInvite(`gelabber.example/invite/${code}/`, origin)).toEqual({
      code,
    });
    expect(parseInvite(`/invite/${code}`, origin)).toEqual({ code });
    expect(parseInvite(`  ${code.toUpperCase()} `, origin)).toEqual({ code });
  });

  it("refuses a link to another Gelabber server", () => {
    const parsed = parseInvite(`https://other.example/invite/${code}`, origin);
    expect(parsed).toEqual({ error: "foreign" });
    expect(inviteInputMessage("foreign")).toBe(
      "Dieser Link gehört zu einem anderen Gelabber-Server.",
    );
    expect(parseInvite(`other.example/invite/${code}`, origin)).toEqual({
      error: "foreign",
    });
  });

  it("reports empty and malformed input", () => {
    expect(parseInvite("   ", origin)).toEqual({ error: "empty" });
    expect(parseInvite("hallo welt", origin)).toEqual({ error: "invalid" });
    expect(parseInvite(`https://gelabber.example/s/${code}`, origin)).toEqual({
      error: "invalid",
    });
    // 0, o, 1, l and i never occur in a code.
    expect(parseInvite("k7xm2pqrstu0", origin)).toEqual({ error: "invalid" });
  });
});

describe("invite expiry", () => {
  const now = Date.parse("2026-10-21T12:00:00Z");
  const at = (ms: number) => new Date(now + ms).toISOString();

  it("says when a link runs out, in words", () => {
    expect(inviteExpiry(at(6 * 86_400_000), now)).toBe("läuft in 6 Tagen ab");
    expect(inviteExpiry(at(86_400_000), now)).toBe("läuft morgen ab");
    expect(inviteExpiry(at(3 * 3_600_000), now)).toBe("läuft in 3 Stunden ab");
    expect(inviteExpiry(at(90_000), now)).toBe("läuft in 2 Minuten ab");
    expect(inviteExpiry(at(10_000), now)).toBe("läuft in 1 Minute ab");
  });

  it("has nothing to say about links without an end", () => {
    expect(inviteExpiry(null, now)).toBeNull();
    expect(inviteExpiry(at(-1), now)).toBe("abgelaufen");
  });
});
