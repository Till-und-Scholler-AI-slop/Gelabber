import { describe, expect, it } from "vitest";

import {
  nextUnread,
  shellShortcut,
  shortcutGroups,
  stepIndex,
} from "./shellShortcuts.ts";
import { callShortcut } from "../voice/pushToTalk.ts";
import { setLocale } from "../i18n/locale.ts";

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

  const all = (mac: boolean, ptt: string | null = null) =>
    shortcutGroups(mac, ptt).flatMap((group) => group.shortcuts);

  it("names the keys as the keyboard prints them", () => {
    expect(all(false)[0]?.keys).toEqual(["Strg", "K"]);
    expect(all(true)[0]?.keys).toEqual(["⌘", "K"]);
  });

  it("lists the call's keys in a group of their own", () => {
    const voice = shortcutGroups(false).find(
      (group) => group.title === "Im Sprachkanal",
    );
    expect(voice?.shortcuts.map((entry) => entry.keys.join("+"))).toEqual([
      "Strg+Umschalt+M",
      "Strg+Umschalt+D",
      "K",
      "M / Leertaste",
    ]);
    // The sheet's keys are the ones callShortcut answers to.
    for (const [code, action] of [
      ["KeyM", "mute"],
      ["KeyD", "deafen"],
    ] as const)
      expect(
        callShortcut(
          {
            code,
            ctrlKey: true,
            metaKey: false,
            shiftKey: true,
            altKey: false,
            repeat: false,
          },
          null,
        ),
      ).toBe(action);
    expect(
      shortcutGroups(true).find((group) => group.title === "Im Sprachkanal")
        ?.shortcuts[0]?.keys,
    ).toEqual(["⌘", "⇧", "M"]);
    // Push-to-talk's key only while it has one.
    expect(
      all(false, "Leertaste").find(
        (entry) => entry.label === "Zum Sprechen gedrückt halten",
      )?.keys,
    ).toEqual(["Leertaste"]);
    expect(
      all(false).some(
        (entry) => entry.label === "Zum Sprechen gedrückt halten",
      ),
    ).toBe(false);
  });

  it("says what Esc does when nothing is open", () => {
    expect(all(false).find((entry) => entry.keys.join() === "Esc")?.label).toBe(
      "Schließen, sonst Unterhaltung als gelesen markieren",
    );
  });

  it("lists Strg+F for the conversation search, not Strg+K (#377)", () => {
    const search = all(false).find(
      (entry) => entry.label === "In der Unterhaltung suchen",
    );
    expect(search?.keys).toEqual(["Strg", "F"]);
    expect(shellShortcut(keys("f", { ctrlKey: true }))).toBeNull();
  });

  it("names keys and shortcuts in English", () => {
    setLocale("en", { persist: false });
    try {
      const [general, voice] = shortcutGroups(false);
      expect(general?.title).toBe("General");
      expect(voice?.title).toBe("In a voice channel");
      const search = general?.shortcuts.find((row) => row.keys.includes("F"));
      expect(search).toEqual({
        keys: ["Ctrl", "F"],
        label: "Search the conversation",
      });
    } finally {
      setLocale("de", { persist: false });
    }
  });
});
