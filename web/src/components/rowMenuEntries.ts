// What a navigation row's menu (RowMenu.tsx) offers, as data: the native
// app lists the same entries (desktop/ui/src/screens/shell/row_menu.rs).
import type { NotifyLevel } from "../messages/notifyLevel.ts";
import { LEVELS, MUTES, levelName, mutedLine } from "./notificationActions.ts";
import type { NotificationTarget } from "./notificationTarget.ts";

export type MenuIcon =
  "check-check" | "bell" | "bell-off" | "gear" | "pencil" | "plus" | "trash";

export type LeafEntry =
  | {
      kind: "item";
      label: string;
      icon?: MenuIcon;
      /** A second line, e.g. how long a mute still holds. */
      hint?: string;
      danger?: boolean;
      /** Shown dimmed and does nothing, e.g. marking read with nothing
       * unread. */
      disabled?: boolean;
      onSelect: () => void;
    }
  | { kind: "radio"; label: string; checked: boolean; onSelect: () => void }
  | { kind: "separator" };

/** An entry of a menu; a submenu holds entries without submenus. */
export type MenuEntry =
  | LeafEntry
  | { kind: "submenu"; label: string; icon?: MenuIcon; entries: LeafEntry[] };

/** The part of `useNotificationActions` the entries call. */
export type NotificationMenuActions = {
  muted: boolean;
  /** Whether anything is unread to mark read. */
  unread: boolean;
  own: { muted_until: string | null };
  inherited: NotifyLevel;
  isLevel: (level: NotifyLevel | null) => boolean;
  markRead: () => Promise<unknown>;
  mute: (minutes?: number) => Promise<unknown>;
  unmute: () => Promise<unknown>;
  setLevel: (level: NotifyLevel | null) => Promise<unknown>;
  openSettings: () => void;
};

/**
 * The notification entries of a server, channel or conversation: mark it
 * read, mute it (or end the mute), its level, and the dialog with all of
 * it. All direct messages together can only be marked read; marking read
 * waits for something unread. `failed`
 * reports a save that did not go through, naming what did not happen.
 */
export function notificationEntries(
  target: NotificationTarget,
  actions: NotificationMenuActions,
  failed: (what: string) => (error: unknown) => void,
): MenuEntry[] {
  const saving = (promise: Promise<unknown>) =>
    void promise.catch(failed("Nicht gespeichert."));
  const entries: MenuEntry[] = [
    {
      kind: "item",
      label: target.allDms
        ? "Alle Direktnachrichten als gelesen markieren"
        : "Als gelesen markieren",
      icon: "check-check",
      disabled: !actions.unread,
      onSelect: () =>
        void actions.markRead().catch(failed("Nicht als gelesen markiert.")),
    },
  ];
  if (target.allDms) return entries;
  entries.push(
    actions.muted
      ? {
          kind: "item",
          label: "Stummschaltung aufheben",
          hint: mutedLine(actions.own.muted_until),
          // Where "Stummschalten" was, with its icon: the levels keep the
          // bell.
          icon: "bell-off",
          onSelect: () => saving(actions.unmute()),
        }
      : {
          kind: "submenu",
          label: "Stummschalten",
          icon: "bell-off",
          entries: MUTES.map((mute) => ({
            kind: "item",
            label: mute.label,
            onSelect: () => saving(actions.mute(mute.minutes)),
          })),
        },
  );
  // A conversation has no level of its own: every message is for you.
  if (!target.dm)
    entries.push({
      kind: "submenu",
      label: "Benachrichtigungen",
      icon: "bell",
      entries: [
        ...(target.serverId
          ? [
              {
                kind: "radio" as const,
                label: `Wie der Server (${levelName(actions.inherited)})`,
                checked: actions.isLevel(null),
                onSelect: () => saving(actions.setLevel(null)),
              },
            ]
          : []),
        ...LEVELS.map((row) => ({
          kind: "radio" as const,
          label: row.label,
          checked: actions.isLevel(row.level),
          onSelect: () => saving(actions.setLevel(row.level)),
        })),
      ],
    });
  entries.push({
    kind: "item",
    label: "Benachrichtigungseinstellungen…",
    icon: "gear",
    onSelect: actions.openSettings,
  });
  return entries;
}
