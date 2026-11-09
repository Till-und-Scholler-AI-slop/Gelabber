// What can be done with the notifications of a server or conversation,
// shared by its row menu (RowMenu.tsx) and the notification dialog: mark it
// read, mute it for a while, pick its level. Each call saves at once.
import {
  settingOf,
  useChangeNotificationSetting,
  useMuteClock,
  useNotificationSettings,
} from "../messages/notificationSettings.ts";
import { isMuted, type NotifyLevel } from "../messages/notifyLevel.ts";
import { useMarkManyRead, useReadState } from "../messages/readState.ts";
import {
  minutesUntil,
  openNotificationSettings,
  type NotificationTarget,
} from "./notificationTarget.ts";

/** How long a mute holds; the last one until it is turned off. `short`
 * fits the dialog's single row of chips. */
export const MUTES: { label: string; short: string; minutes?: number }[] = [
  { label: "15 Minuten", short: "15 Min.", minutes: 15 },
  { label: "1 Stunde", short: "1 Std.", minutes: 60 },
  { label: "8 Stunden", short: "8 Std.", minutes: 8 * 60 },
  { label: "24 Stunden", short: "24 Std.", minutes: 24 * 60 },
  { label: "Bis ich es wieder einschalte", short: "Unbegrenzt" },
];

export const LEVELS: { level: NotifyLevel; label: string; hint: string }[] = [
  { level: "all", label: "Alle Nachrichten", hint: "Jede neue Nachricht" },
  {
    level: "mentions",
    label: "Nur @Erwähnungen",
    hint: "Wenn dich jemand erwähnt oder @everyone/@here schreibt",
  },
  {
    level: "none",
    label: "Nichts",
    hint: "Keine Benachrichtigungen, kein Zähler",
  },
];

export const levelName = (level: NotifyLevel) =>
  LEVELS.find((row) => row.level === level)!.label;

/** "14:30 Uhr" today, else "Mi., 5. Nov., 14:30 Uhr". */
export function formatUntil(until: string): string {
  const date = new Date(until);
  const today = new Date();
  const time = date.toLocaleTimeString("de-DE", {
    hour: "2-digit",
    minute: "2-digit",
  });
  return date.toDateString() === today.toDateString()
    ? `${time} Uhr`
    : `${date.toLocaleDateString("de-DE", { weekday: "short", day: "numeric", month: "short" })}, ${time} Uhr`;
}

/** The running mute in words. */
export function mutedLine(mutedUntil: string | null): string {
  return mutedUntil
    ? `Stumm bis ${formatUntil(mutedUntil)}`
    : "Stumm, bis du es aufhebst";
}

export function useNotificationActions(target: NotificationTarget) {
  const { data: settings } = useNotificationSettings();
  const change = useChangeNotificationSetting();
  const markMany = useMarkManyRead();
  const { data: reads } = useReadState();
  const now = useMuteClock();
  const own = settingOf(settings, target.scope, target.id);
  const server = target.serverId
    ? settingOf(settings, "server", target.serverId)
    : null;
  return {
    own,
    muted: isMuted(own, now),
    /** Something to mark read, muted or not. */
    unread: (reads ?? []).some(
      (row) =>
        row.unread_count > 0 &&
        (target.allDms
          ? row.server_id === null
          : target.scope === "server"
            ? row.server_id === target.id
            : row.channel_id === target.id),
    ),
    serverMuted: server ? isMuted(server, now) : false,
    /** What "Wie der Server" means for a channel. */
    inherited: (server?.level ?? "all") as NotifyLevel,
    /** Whether a level choice is the one set; `null` is "Wie der Server". */
    isLevel: (level: NotifyLevel | null) =>
      target.serverId ? own.level === level : (own.level ?? "all") === level,
    mute: (minutes?: number) =>
      change(target.scope, target.id, {
        level: own.level,
        muted: true,
        ...(minutes ? { mute_minutes: minutes } : {}),
      }),
    unmute: () =>
      change(target.scope, target.id, { level: own.level, muted: false }),
    /** A new level keeps a running mute and its end. */
    setLevel: (level: NotifyLevel | null) =>
      change(target.scope, target.id, {
        // A server at "all" needs no row of its own.
        level: !target.serverId && level === "all" ? null : level,
        muted: own.muted,
        ...(own.muted && own.muted_until
          ? { mute_minutes: minutesUntil(own.muted_until, now) }
          : {}),
      }),
    markRead: () =>
      markMany(
        target.allDms
          ? { all: "dms" }
          : target.scope === "server"
            ? { server: target.id }
            : { channel: target.id },
      ),
    openSettings: () => openNotificationSettings(target),
  };
}
