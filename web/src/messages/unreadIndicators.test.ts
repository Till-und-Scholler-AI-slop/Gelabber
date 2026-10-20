import { describe, expect, it, vi } from "vitest";

import { setAppBadge, titleWith, unreadMark } from "./unreadIndicators.ts";

describe("unread indicators", () => {
  it("counts mentions and direct messages, dots the rest", () => {
    const row = (
      channel_id: string,
      server_id: string | null,
      unread_count: number,
      mention_count = 0,
    ) => ({ channel_id, server_id, unread_count, mention_count });
    const rows = [
      row("dm", null, 2),
      row("general", "s", 5, 1),
      row("quiet", "s", 3),
      row("read", "s", 0),
    ];
    expect(unreadMark(rows, null, 0)).toEqual({ count: 3, any: true });
    expect(unreadMark(rows, "dm", 0)).toEqual({ count: 1, any: true });
    // Only a dot left: unread, but nothing to count.
    expect(unreadMark([row("quiet", "s", 3)], null, 0)).toEqual({
      count: 0,
      any: true,
    });
    expect(unreadMark([row("dm", null, 4)], "dm", 0)).toEqual({
      count: 0,
      any: false,
    });
    // A muted conversation counts for nothing, until the mute ends.
    const muted = {
      ...row("dm", null, 4),
      muted: true,
      muted_until: "1970-01-01T00:01:00Z",
    };
    expect(unreadMark([muted], null, 0)).toEqual({ count: 0, any: false });
    expect(unreadMark([muted], null, 120_000)).toEqual({ count: 4, any: true });
    expect(unreadMark([], null, 0)).toEqual({ count: 0, any: false });
  });

  it("formats the tab title", () => {
    const mark = (count: number) => ({ count, any: count > 0 });
    expect(titleWith("Gelabber", mark(0))).toBe("Gelabber");
    expect(titleWith("Gelabber", mark(1))).toBe("(1) Gelabber");
    expect(titleWith("Gelabber", mark(99))).toBe("(99) Gelabber");
    expect(titleWith("Gelabber", mark(100))).toBe("(99+) Gelabber");
    expect(titleWith("Gelabber", mark(250))).toBe("(99+) Gelabber");
    // Its own mark is replaced, never stacked, and taken off at 0.
    expect(titleWith("(3) Gelabber", mark(4))).toBe("(4) Gelabber");
    expect(titleWith("(99+) Gelabber", mark(0))).toBe("Gelabber");
    expect(titleWith("• Gelabber", { count: 0, any: true })).toBe("• Gelabber");
    expect(titleWith("• Gelabber", mark(2))).toBe("(2) Gelabber");
    // A title a page set keeps its text.
    expect(titleWith("Einstellungen · Gelabber", mark(2))).toBe(
      "(2) Einstellungen · Gelabber",
    );
  });

  it("sets and clears the app badge where the browser has one", async () => {
    const nav = {
      setAppBadge: vi.fn(() => Promise.resolve()),
      clearAppBadge: vi.fn(() => Promise.resolve()),
    };
    setAppBadge(3, nav);
    setAppBadge(0, nav);
    expect(nav.setAppBadge).toHaveBeenCalledWith(3);
    expect(nav.clearAppBadge).toHaveBeenCalledOnce();
    // No API, a refusing one: nothing thrown.
    setAppBadge(2, {});
    setAppBadge(2, undefined);
    setAppBadge(2, {
      setAppBadge: () => Promise.reject(new Error("not installed")),
    });
    setAppBadge(2, {
      setAppBadge: () => {
        throw new Error("insecure");
      },
    });
  });
});
