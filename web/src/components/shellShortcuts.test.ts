import { describe, expect, it } from "vitest";

import {
  nextUnread,
  shellShortcut,
  shortcutList,
  stepIndex,
} from "./shellShortcuts.ts";

const keys = (
  key: string,
  mods: Partial<
    Record<"altKey" | "ctrlKey" | "metaKey" | "shiftKey", boolean>
  > = {},
) => ({
  key,
  altKey: false,
  ctrlKey: false,
  metaKey: false,
  shiftKey: false,
  ...mods,
});

describe("shell shortcuts", () => {
  it("opens the switcher with Strg+K or ⌘+K", () => {
    expect(shellShortcut(keys("k", { ctrlKey: true }))).toEqual({
      kind: "switcher",
    });
    expect(shellShortcut(keys("K", { metaKey: true }))).toEqual({
      kind: "switcher",
    });
    expect(shellShortcut(keys("k"))).toBeNull();
    expect(shellShortcut(keys("k", { ctrlKey: true, shiftKey: true }))).toBe(
      null,
    );
  });

  it("opens the shortcut list with Strg+/, with or without Umschalt", () => {
    expect(shellShortcut(keys("/", { ctrlKey: true }))).toEqual({
      kind: "help",
    });
    expect(shellShortcut(keys("/", { ctrlKey: true, shiftKey: true }))).toEqual(
      { kind: "help" },
    );
  });

  it("steps through channels, unread channels and servers with Alt", () => {
    expect(shellShortcut(keys("ArrowDown", { altKey: true }))).toEqual({
      kind: "channel",
      step: 1,
    });
    expect(
      shellShortcut(keys("ArrowUp", { altKey: true, shiftKey: true })),
    ).toEqual({ kind: "unread", step: -1 });
    expect(
      shellShortcut(keys("ArrowDown", { altKey: true, ctrlKey: true })),
    ).toEqual({ kind: "server", step: 1 });
    expect(shellShortcut(keys("ArrowDown"))).toBeNull();
    expect(shellShortcut(keys("ArrowLeft", { altKey: true }))).toBeNull();
  });

  it("stops at the ends of a list", () => {
    expect(stepIndex(3, 1, 1)).toBe(2);
    expect(stepIndex(3, 2, 1)).toBeNull();
    expect(stepIndex(3, 0, -1)).toBeNull();
    expect(stepIndex(3, -1, 1)).toBe(0);
    expect(stepIndex(3, -1, -1)).toBe(2);
    expect(stepIndex(0, -1, 1)).toBeNull();
  });

  it("finds the next unread entry round the end of the list", () => {
    const unread = [true, false, false, true];
    expect(nextUnread(unread, 0, 1)).toBe(3);
    expect(nextUnread(unread, 3, 1)).toBe(0);
    expect(nextUnread(unread, 0, -1)).toBe(3);
    expect(nextUnread([true, false], 0, 1)).toBeNull();
    expect(nextUnread([false, true], -1, 1)).toBe(1);
  });

  it("names the keys as the keyboard prints them", () => {
    expect(shortcutList(false)[0]?.keys).toEqual(["Strg", "K"]);
    expect(shortcutList(true)[0]?.keys).toEqual(["⌘", "K"]);
  });
});
