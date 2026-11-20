import { beforeAll, describe, expect, it, vi } from "vitest";

import {
  COLLAPSE_BELOW,
  WIDTH_MAX,
  WIDTH_MIN,
  clampWidth,
  dragWidth,
  edgeKey,
  fitWidths,
  memberListShown,
  setChannelListCollapsed,
  setSidebarWidth,
  sidebarLayout,
  toggleChannelList,
} from "./sidebarLayout.ts";

beforeAll(() => {
  const items = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => items.set(key, value),
    removeItem: (key: string) => items.delete(key),
  });
});

const wide = {
  sidebar: 224,
  members: 248,
  collapsed: false,
  membersShown: true,
};

describe("sidebar widths", () => {
  it("keeps widths between the minimum and the maximum", () => {
    expect(clampWidth(120)).toBe(WIDTH_MIN);
    expect(clampWidth(999)).toBe(WIDTH_MAX);
    expect(clampWidth(250.4)).toBe(250);
  });

  it("remembers a width per side and goes back to the default", () => {
    setSidebarWidth("channels", 300);
    setSidebarWidth("members", 500);
    expect(sidebarLayout().sidebar).toBe(300);
    expect(sidebarLayout().members).toBe(WIDTH_MAX);
    expect(localStorage.getItem("gelabber.sidebar.width")).toBe("300");
    setSidebarWidth("channels", null);
    setSidebarWidth("members", null);
    expect(sidebarLayout().sidebar).toBeNull();
    expect(localStorage.getItem("gelabber.members.width")).toBeNull();
  });

  it("folds the channel list away and back, remembered", () => {
    toggleChannelList();
    expect(sidebarLayout().collapsed).toBe(true);
    expect(localStorage.getItem("gelabber.sidebar.collapsed")).toBe("1");
    setChannelListCollapsed(false);
    expect(sidebarLayout().collapsed).toBe(false);
    expect(localStorage.getItem("gelabber.sidebar.collapsed")).toBeNull();
  });

  it("counts the member lists shown beside the page", () => {
    const first = memberListShown();
    const second = memberListShown();
    expect(sidebarLayout().membersShown).toBe(2);
    first();
    second();
    expect(sidebarLayout().membersShown).toBe(0);
  });
});

describe("fitting the columns to the window", () => {
  it("leaves the chosen widths where the page has room", () => {
    expect(fitWidths({ ...wide, viewport: 1440 })).toEqual({
      sidebar: 224,
      members: 248,
    });
  });

  it("narrows the member list first, then the channel list", () => {
    // 64 + 360 + 360 + 440 = 1224: 64 too many at 1160.
    expect(
      fitWidths({ ...wide, sidebar: 360, members: 360, viewport: 1160 }),
    ).toEqual({ sidebar: 360, members: 296 });
    // At 1131 the member list is at its minimum and the channel list
    // gives the rest: 1131 - 64 - 200 - 440 = 427, so 360 fits.
    expect(
      fitWidths({ ...wide, sidebar: 360, members: 360, viewport: 1131 }),
    ).toEqual({ sidebar: 360, members: 267 });
  });

  it("counts only columns beside the page", () => {
    // The member list is a drawer up to 1130: only the channel list counts.
    expect(
      fitWidths({ ...wide, sidebar: 360, members: 360, viewport: 820 }),
    ).toEqual({ sidebar: 316, members: 360 });
    // Folded, the channel list takes no room.
    expect(
      fitWidths({
        ...wide,
        sidebar: 360,
        members: 360,
        collapsed: true,
        viewport: 1131,
      }),
    ).toEqual({ sidebar: 360, members: 360 });
    // Hidden members leave the room to the channel list.
    expect(
      fitWidths({
        ...wide,
        sidebar: 360,
        members: 360,
        membersShown: false,
        viewport: 1131,
      }),
    ).toEqual({ sidebar: 360, members: 360 });
  });

  it("never goes under the minimum, even when the page gets less", () => {
    expect(
      fitWidths({ ...wide, sidebar: 360, members: 360, viewport: 801 }),
    ).toEqual({ sidebar: 297, members: 360 });
  });
});

describe("the edge", () => {
  it("moves with the arrow keys the way the edge goes", () => {
    expect(edgeKey("channels", "ArrowRight", 224)).toBe(240);
    expect(edgeKey("channels", "ArrowLeft", 224)).toBe(208);
    // On the right-hand list, Left moves the edge left: wider.
    expect(edgeKey("members", "ArrowLeft", 248)).toBe(264);
    expect(edgeKey("members", "ArrowRight", 248)).toBe(232);
    expect(edgeKey("members", "Home", 248)).toBe(WIDTH_MIN);
    expect(edgeKey("channels", "End", 248)).toBe(WIDTH_MAX);
    expect(edgeKey("channels", "Enter", 248)).toBe("collapse");
    expect(edgeKey("channels", "a", 248)).toBeNull();
  });

  it("follows a drag and folds well under the minimum", () => {
    expect(dragWidth("channels", 224, 40)).toBe(264);
    expect(dragWidth("members", 248, 40)).toBe(208);
    // Just under the minimum it holds there.
    expect(dragWidth("channels", 224, WIDTH_MIN - 224 - 10)).toBe(WIDTH_MIN);
    expect(dragWidth("channels", 224, COLLAPSE_BELOW - 224 - 1)).toBe(
      "collapse",
    );
    expect(dragWidth("members", 248, 248 - COLLAPSE_BELOW + 1)).toBe(
      "collapse",
    );
  });
});
