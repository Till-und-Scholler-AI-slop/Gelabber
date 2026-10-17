import { describe, expect, it } from "vitest";

import {
  badgeOf,
  isMuted,
  sumBadges,
  wantsNotification,
} from "./notifyLevel.ts";

const NOW = Date.parse("2026-10-17T12:00:00Z");
const later = "2026-10-17T13:00:00Z";
const earlier = "2026-10-17T11:00:00Z";
const channel = (over: object = {}) => ({
  channel_id: "c",
  server_id: "s",
  unread_count: 3,
  mention_count: 0,
  ...over,
});
const dm = (over: object = {}) => ({
  channel_id: "d",
  server_id: null,
  unread_count: 2,
  ...over,
});

describe("notification levels", () => {
  it("ends a mute on the clock", () => {
    expect(isMuted({ muted: true, muted_until: later }, NOW)).toBe(true);
    expect(isMuted({ muted: true, muted_until: earlier }, NOW)).toBe(false);
    expect(isMuted({ muted: true, muted_until: null }, NOW)).toBe(true);
    expect(isMuted({ muted: false }, NOW)).toBe(false);
    expect(isMuted(undefined, NOW)).toBe(false);
  });

  it("shows a dot for ordinary unread and a number for mentions", () => {
    expect(badgeOf(channel(), null, NOW)).toEqual({ count: 0, dot: true });
    expect(badgeOf(channel({ mention_count: 2 }), null, NOW)).toEqual({
      count: 2,
      dot: true,
    });
    expect(badgeOf(channel({ unread_count: 0 }), null, NOW)).toEqual({
      count: 0,
      dot: false,
    });
    // DMs always count.
    expect(badgeOf(dm(), null, NOW)).toEqual({ count: 2, dot: true });
    // The conversation being read shows nothing.
    expect(badgeOf(channel({ mention_count: 1 }), "c", NOW)).toEqual({
      count: 0,
      dot: false,
    });
  });

  it("hides muted unread but keeps mentions unless the level is nothing", () => {
    const muted = { muted: true, muted_until: null };
    expect(badgeOf(channel({ ...muted, mention_count: 1 }), null, NOW)).toEqual(
      { count: 1, dot: false },
    );
    expect(badgeOf(channel(muted), null, NOW)).toEqual({
      count: 0,
      dot: false,
    });
    expect(
      badgeOf(channel({ level: "none", mention_count: 4 }), null, NOW),
    ).toEqual({ count: 0, dot: false });
    expect(badgeOf(dm(muted), null, NOW)).toEqual({ count: 0, dot: false });
    // An expired mute is no mute.
    expect(
      badgeOf(channel({ muted: true, muted_until: earlier }), null, NOW),
    ).toEqual({ count: 0, dot: true });
  });

  it("adds badges up", () => {
    expect(
      sumBadges([
        { count: 1, dot: false },
        { count: 0, dot: true },
        { count: 2, dot: false },
      ]),
    ).toEqual({ count: 3, dot: true });
    expect(sumBadges([])).toEqual({ count: 0, dot: false });
  });

  it("announces by level and mute", () => {
    const plain = { dm: false, mentioned: false };
    const mention = { dm: false, mentioned: true };
    expect(wantsNotification(undefined, plain, NOW)).toBe(true);
    expect(wantsNotification({ level: "all" }, plain, NOW)).toBe(true);
    expect(wantsNotification({ level: "mentions" }, plain, NOW)).toBe(false);
    expect(wantsNotification({ level: "mentions" }, mention, NOW)).toBe(true);
    expect(
      wantsNotification(
        { level: "mentions" },
        { dm: true, mentioned: false },
        NOW,
      ),
    ).toBe(true);
    expect(wantsNotification({ level: "none" }, mention, NOW)).toBe(false);
    expect(
      wantsNotification({ muted: true, muted_until: later }, mention, NOW),
    ).toBe(false);
    expect(
      wantsNotification({ muted: true, muted_until: earlier }, plain, NOW),
    ).toBe(true);
  });
});
