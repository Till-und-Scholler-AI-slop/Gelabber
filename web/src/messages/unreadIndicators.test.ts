import { describe, expect, it, vi } from "vitest";

import { setAppBadge, titleWith, unreadMark } from "./unreadIndicators.ts";

describe("unread indicators", () => {
  it("counts every conversation but the one being read", () => {
    const rows = [
      { channel_id: "a", unread_count: 2 },
      { channel_id: "b", unread_count: 1 },
      { channel_id: "c", unread_count: 0 },
    ];
    expect(unreadMark(rows, null)).toEqual({ count: 3, any: true });
    expect(unreadMark(rows, "a")).toEqual({ count: 1, any: true });
    expect(unreadMark([{ channel_id: "a", unread_count: 4 }], "a")).toEqual({
      count: 0,
      any: false,
    });
    expect(unreadMark([], null)).toEqual({ count: 0, any: false });
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
