import { describe, expect, it } from "vitest";

import { badgeSeen } from "./badgeEntrance.ts";

describe("badge entrance", () => {
  it("is no news for a badge that is there from the start", () => {
    expect(badgeSeen(null, true, true).fresh).toBe(false);
  });

  it("is no news for a badge that comes with the first read state", () => {
    const mounted = badgeSeen(null, false, false);
    expect(badgeSeen(mounted, true, true).fresh).toBe(false);
  });

  it("is news when it appears on a loaded list, until it goes", () => {
    const quiet = badgeSeen(null, true, false);
    const appeared = badgeSeen(quiet, true, true);
    expect(appeared.fresh).toBe(true);
    expect(badgeSeen(appeared, true, true).fresh).toBe(true);
    const read = badgeSeen(appeared, true, false);
    expect(read.fresh).toBe(false);
    expect(badgeSeen(read, true, true).fresh).toBe(true);
  });
});
