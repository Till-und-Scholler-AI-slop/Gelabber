// What can be done with the notifications of a server or conversation,
// shared by its row menu (RowMenu.tsx) and the notification dialog: mark it
// read, mute it for a while, pick its level. Each call saves at once.
import { translate } from "../i18n/catalog.ts";
import { notifications } from "../i18n/catalogs/notifications.ts";
import { formatDate, formatTime } from "../i18n/format.ts";
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

type MuteKey = "quarterHour" | "hour" | "eightHours" | "day" | "forever";

/** A mute choice whose texts are read in the language shown at the time. */
function mute(key: MuteKey, minutes?: number) {
  return {
    get label() {
      return translate(notifications, `mute.${key}`);
    },
    get short() {
      return translate(notifications, `mute.${key}.short`);
    },
    ...(minutes ? { minutes } : {}),
  };
}

/** How long a mute holds; the last one until it is turned off. `short`
 * fits the dialog's single row of chips. */
export const MUTES: { label: string; short: string; minutes?: number }[] = [
  mute("quarterHour", 15),
  mute("hour", 60),
  mute("eightHours", 8 * 60),
  mute("day", 24 * 60),
  mute("forever"),
];

/** A level choice whose texts are read in the language shown at the time. */
function level(id: NotifyLevel) {
  return {
    level: id,
    get label() {
      return translate(notifications, `level.${id}`);
    },
    get hint() {
      return translate(notifications, `level.${id}.hint`);
    },
  };
}

export const LEVELS: { level: NotifyLevel; label: string; hint: string }[] = [
  level("all"),
  level("mentions"),
  level("none"),
];

export const levelName = (level: NotifyLevel) =>
  LEVELS.find((row) => row.level === level)!.label;

/** "14:30 Uhr" today, else "Mi., 5. Nov., 14:30 Uhr" ("14:30", "Wed 5 Nov,
 * 14:30" in English). */
export function formatUntil(until: string): string {
  const date = new Date(until);
  const today = new Date();
  const time = formatTime(date, { hour: "2-digit", minute: "2-digit" });
  return date.toDateString() === today.toDateString()
    ? translate(notifications, "until.today", { time })
    : translate(notifications, "until.later", {
        date: formatDate(date, {
          weekday: "short",
          day: "numeric",
          month: "short",
        }),
        time,
      });
}

/** The running mute in words. */
export function mutedLine(mutedUntil: string | null): string {
  return mutedUntil
    ? translate(notifications, "muted.until", {
        until: formatUntil(mutedUntil),
      })
    : translate(notifications, "muted.untilUnmuted");
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
