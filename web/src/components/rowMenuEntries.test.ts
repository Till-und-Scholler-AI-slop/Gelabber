import { describe, expect, it, vi } from "vitest";

import type { NotifyLevel } from "../messages/notifyLevel.ts";
import {
  notificationEntries,
  type MenuEntry,
  type NotificationMenuActions,
} from "./rowMenuEntries.ts";
import { isMenuKey } from "./rowMenuTrigger.ts";

function actions(
  over: Partial<NotificationMenuActions> = {},
): NotificationMenuActions {
  return {
    muted: false,
    own: { muted_until: null },
    inherited: "mentions",
    isLevel: (level: NotifyLevel | null) => level === null,
    markRead: vi.fn(() => Promise.resolve()),
    mute: vi.fn(() => Promise.resolve()),
    unmute: vi.fn(() => Promise.resolve()),
    setLevel: vi.fn(() => Promise.resolve()),
    openSettings: vi.fn(),
    ...over,
  };
}

const outline = (entries: MenuEntry[]) =>
  entries.map((entry) =>
    entry.kind === "separator"
      ? "—"
      : entry.kind === "submenu"
        ? `${entry.label} ▸ ${entry.entries
            .map((child) =>
              child.kind === "radio"
                ? `${child.checked ? "(•)" : "( )"} ${child.label}`
                : child.kind === "item"
                  ? child.label
                  : "—",
            )
            .join(" | ")}`
        : entry.label,
  );

const channel = {
  scope: "channel" as const,
  id: "c1",
  name: "#spiele",
  serverId: "s1",
};
const noReport = () => () => {};

describe("row menu entries", () => {
  it("offers a channel: read, mute, its level as the server's or its own, the dialog", () => {
    expect(outline(notificationEntries(channel, actions(), noReport))).toEqual([
      "Als gelesen markieren",
      "Stummschalten ▸ 15 Minuten | 1 Stunde | 8 Stunden | 24 Stunden | Bis ich es wieder einschalte",
      "Benachrichtigungen ▸ (•) Wie der Server (Nur @Erwähnungen) | ( ) Alle Nachrichten | ( ) Nur @Erwähnungen | ( ) Nichts",
      "Benachrichtigungseinstellungen…",
    ]);
  });

  it("gives a server its own levels, all messages by default", () => {
    const server = { scope: "server" as const, id: "s1", name: "Wohnzimmer" };
    const entries = notificationEntries(
      server,
      actions({ isLevel: (level) => level === "all" }),
      noReport,
    );
    expect(outline(entries)[2]).toBe(
      "Benachrichtigungen ▸ (•) Alle Nachrichten | ( ) Nur @Erwähnungen | ( ) Nichts",
    );
  });

  it("has no level for a conversation, and only mark-read for all of them", () => {
    const dm = { scope: "channel" as const, id: "d1", name: "Jonas", dm: true };
    expect(outline(notificationEntries(dm, actions(), noReport))).toEqual([
      "Als gelesen markieren",
      "Stummschalten ▸ 15 Minuten | 1 Stunde | 8 Stunden | 24 Stunden | Bis ich es wieder einschalte",
      "Benachrichtigungseinstellungen…",
    ]);
    const all = { ...dm, id: "dms", name: "Direktnachrichten", allDms: true };
    expect(outline(notificationEntries(all, actions(), noReport))).toEqual([
      "Alle als gelesen markieren",
    ]);
  });

  it("ends a running mute in one step and says until when", () => {
    const muted = actions({ muted: true });
    const [, unmute] = notificationEntries(channel, muted, noReport);
    expect(unmute).toMatchObject({
      kind: "item",
      label: "Stummschaltung aufheben",
      hint: "Stumm, bis du es aufhebst",
    });
    if (unmute?.kind === "item") unmute.onSelect();
    expect(muted.unmute).toHaveBeenCalled();
  });

  it("saves the picked length and level, and reports a failed save", async () => {
    const report = vi.fn();
    const failing = actions({
      setLevel: vi.fn(() => Promise.reject(new Error("offline"))),
    });
    const entries = notificationEntries(
      channel,
      failing,
      (what) => (error) => report(what, error),
    );
    const mute = entries[1];
    const level = entries[2];
    if (mute?.kind !== "submenu" || level?.kind !== "submenu")
      throw new Error("submenus expected");
    const hour = mute.entries[1];
    if (hour?.kind === "item") hour.onSelect();
    expect(failing.mute).toHaveBeenCalledWith(60);
    const forever = mute.entries[4];
    if (forever?.kind === "item") forever.onSelect();
    expect(failing.mute).toHaveBeenLastCalledWith(undefined);
    const nothing = level.entries[3];
    if (nothing?.kind === "radio") nothing.onSelect();
    expect(failing.setLevel).toHaveBeenCalledWith("none");
    await Promise.resolve();
    expect(report).toHaveBeenCalledWith(
      "Nicht gespeichert.",
      expect.any(Error),
    );
  });
});

describe("menu keys", () => {
  it("open the menu with Umschalt+F10 or the context-menu key only", () => {
    expect(isMenuKey({ key: "F10", shiftKey: true })).toBe(true);
    expect(isMenuKey({ key: "ContextMenu", shiftKey: false })).toBe(true);
    expect(isMenuKey({ key: "F10", shiftKey: false })).toBe(false);
    expect(isMenuKey({ key: "Enter", shiftKey: true })).toBe(false);
  });
});
