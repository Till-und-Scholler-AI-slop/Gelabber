import { describe, expect, it } from "vitest";

import {
  badgeOf,
  isMuted,
  sumBadges,
  wantsNotification,
} from "./notifyLevel.ts";
import {
  conversationSetting,
  type NotificationSetting,
} from "./notificationSettings.ts";

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

describe("a conversation without an unread row", () => {
  const setting = (
    scope: "server" | "channel",
    id: string,
    change: Partial<NotificationSetting>,
  ): NotificationSetting => ({
    scope,
    id,
    level: null,
    muted: false,
    muted_until: null,
    ...change,
  });

  it("follows its server's own setting", () => {
    const settings = [setting("server", "s1", { muted: true })];
    const muted = conversationSetting(settings, "c1", "s1", 0);
    expect(wantsNotification(muted, { dm: false, mentioned: false }, 0)).toBe(
      false,
    );
    expect(conversationSetting(settings, "c1", "s2", 0)).toEqual({
      level: "all",
      muted: false,
      muted_until: null,
    });
    expect(conversationSetting(undefined, "c1", "s1", 0)).toBeUndefined();
  });

  it("takes its own setting before its server's", () => {
    const settings = [
      setting("server", "s1", { level: "none" }),
      setting("channel", "c1", { level: "all" }),
      setting("channel", "c2", { muted: true }),
      setting("channel", "dm", { level: "mentions" }),
    ];
    const plain = { dm: false, mentioned: false };
    expect(conversationSetting(settings, "c1", "s1", 0)?.level).toBe("all");
    expect(
      wantsNotification(conversationSetting(settings, "c1", "s1", 0), plain, 0),
    ).toBe(true);
    // Inheriting the level, muted by itself.
    expect(conversationSetting(settings, "c2", "s1", 0)).toEqual({
      level: "none",
      muted: true,
      muted_until: null,
    });
    // A DM has no server, only its own setting.
    expect(conversationSetting(settings, "dm", null, 0)?.level).toBe(
      "mentions",
    );
  });

  it("is muted until the later of two running mutes", () => {
    const soon = new Date(NOW + 60_000).toISOString();
    const later = new Date(NOW + 3_600_000).toISOString();
    const gone = new Date(NOW - 60_000).toISOString();
    const both = [
      setting("server", "s1", { muted: true, muted_until: soon }),
      setting("channel", "c1", { muted: true, muted_until: later }),
    ];
    expect(conversationSetting(both, "c1", "s1", NOW)).toMatchObject({
      muted: true,
      muted_until: later,
    });
    const expired = [
      setting("server", "s1", { muted: true, muted_until: soon }),
      setting("channel", "c1", { muted: true, muted_until: gone }),
    ];
    expect(conversationSetting(expired, "c1", "s1", NOW)).toMatchObject({
      muted: true,
      muted_until: soon,
    });
  });
});
