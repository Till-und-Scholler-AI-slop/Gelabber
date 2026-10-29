// A notice when someone else goes live in one of my servers. The server used
// to post a chat row for that (`live_started`); without the row, this gives
// the same popup, system notification and cue, but no row and no unread dot.
// It follows the message switches (toasts, desktop notifications, the message
// cue), the server's mute and level, and never sounds into a deafened call.
// useLiveNotices.ts feeds it the gateway's voice frames.

import type { NotificationSetting } from "../messages/notificationSettings.ts";
import {
  conversationPath,
  createNotificationPacing,
  messageNotificationDecision,
} from "../messages/notify.ts";
import { wantsNotification } from "../messages/notifyLevel.ts";
import type { MessageToast } from "../messages/toasts.ts";
import type { MessageNotification } from "../pwa/notifications.ts";
import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import type { SigEvent, VoiceEntry } from "../ws/protocol.ts";
import { liveSentence } from "./roomSummary.ts";

/** Once per person and voice channel until they have been off Live there
 * this long, as the server held its chat notice. */
export const LIVE_NOTICE_COOLDOWN_MS = 10 * 60_000;
/** The server sends a roster and a start event in either order: a Live the
 * roster showed only this recently is still new to the event. */
export const LIVE_ROSTER_GRACE_MS = 5_000;

type Seen = {
  live: boolean;
  /** When an end event took the Live away; its cooldown runs from there. */
  endedAt: number | null;
  /** Live only as far as a roster says, since then. */
  rosterAt: number | null;
};

/**
 * Which start events begin a Live worth a notice on this device: not one it
 * already knows of (from an event or a roster), and not one back within the
 * cooldown after its end, as when a reconnect drops and restarts it. A Live
 * this device only ever saw in a roster (opened while it ran) is never new.
 */
export function createLiveStarts(
  cooldownMs = LIVE_NOTICE_COOLDOWN_MS,
  graceMs = LIVE_ROSTER_GRACE_MS,
) {
  const servers = new Map<string, Map<string, Seen>>();
  const seatsOf = (serverId: string) => {
    let seats = servers.get(serverId);
    if (!seats) servers.set(serverId, (seats = new Map()));
    return seats;
  };
  const keyOf = (channelId: string, userId: string) => `${channelId} ${userId}`;
  return {
    /** A start event (`p` of a screen). */
    start(
      serverId: string,
      channelId: string,
      userId: string,
      now: number,
    ): boolean {
      const seats = seatsOf(serverId);
      const key = keyOf(channelId, userId);
      const seen = seats.get(key);
      const known =
        seen?.live === true &&
        (seen.rosterAt === null || now - seen.rosterAt >= graceMs);
      const recent =
        seen !== undefined &&
        seen.endedAt !== null &&
        now - seen.endedAt < cooldownMs;
      seats.set(key, {
        live: true,
        endedAt: seen?.endedAt ?? null,
        rosterAt: null,
      });
      return !known && !recent;
    },
    /** An end event: `u` of a screen, or `l` out of the channel. */
    end(serverId: string, channelId: string, userId: string, now: number) {
      const seats = servers.get(serverId);
      const key = keyOf(channelId, userId);
      if (seats?.get(key)?.live)
        seats.set(key, { live: false, endedAt: now, rosterAt: null });
    },
    /** A roster (`r`): on subscribe, after a reconnect, or as it changes. */
    roster(serverId: string, entries: readonly VoiceEntry[], now: number) {
      const seats = seatsOf(serverId);
      const live = new Set(
        entries.flatMap((entry) => (entry.s ? [keyOf(entry.c, entry.u)] : [])),
      );
      for (const [key, seen] of seats) {
        if (seen.live && live.has(key)) continue;
        // Gone without an end event (missed while offline): when it ended is
        // unknown, so it starts no cooldown; an earlier end keeps its own.
        const ended = { ...seen, live: false, rosterAt: null };
        if (ended.endedAt === null || now - ended.endedAt >= cooldownMs)
          seats.delete(key);
        else seats.set(key, ended);
      }
      for (const key of live) {
        const seen = seats.get(key);
        if (!seen?.live)
          seats.set(key, {
            live: true,
            endedAt: seen?.endedAt ?? null,
            rosterAt: now,
          });
      }
    },
  };
}

export type LiveStart = { serverId: string; channelId: string; userId: string };

/** What the device and window say about a notice. */
export type LiveDevice = {
  /** The `messageToasts` switch (voice/settings.ts). */
  toasts: boolean;
  /** The `desktopNotify` switch. */
  desktop: boolean;
  hidden: boolean;
  focused: boolean;
  /** The channel this window shows. */
  viewingChannelId: string | undefined;
};

/**
 * Whether a new Live is announced here, and how. Not my own, and not in the
 * voice channel I sit in (on this device or another): the stage shows it
 * there. Only for a voice channel of the server I may join. A voice channel
 * has no notification setting of its own, so the server's decides; a Live
 * mentions nobody, so it is announced at the level "all" only, as the chat
 * notice in a channel of that server was. The toast and the system
 * notification then follow the rules of a message in that voice channel.
 */
export function liveNoticeDecision(input: {
  start: LiveStart;
  me: string;
  here: boolean;
  server: Pick<ServerDetail, "role" | "permissions" | "channels"> | undefined;
  /** My setting for that server (notificationSettings.ts `settingOf`). */
  setting: Pick<NotificationSetting, "level" | "muted" | "muted_until">;
  /** Server clock, for a mute with an end. */
  now: number;
  device: LiveDevice;
}): { toast: boolean; desktop: boolean } {
  const { start, server, setting, device } = input;
  const none = { toast: false, desktop: false };
  if (start.userId === input.me || input.here) return none;
  const voice = server?.channels.some(
    (channel) => channel.id === start.channelId && channel.kind === "voice",
  );
  if (!voice || !can(server, "join_voice")) return none;
  const level = { ...setting, level: setting.level ?? undefined };
  if (!wantsNotification(level, { dm: false, mentioned: false }, input.now))
    return none;
  return messageNotificationDecision({
    toastEnabled: device.toasts,
    desktopEnabled: device.desktop,
    hidden: device.hidden,
    focused: device.focused,
    type: "c",
    own: false,
    channelId: start.channelId,
    viewingChannelId: device.viewingChannelId,
  });
}

/** Who showed a system notification (pwa/notifications.ts). */
export type Shown = "worker" | "page" | "app" | "viewed" | "none";

/** As with a message: a desktop plays the cue with a notification this
 * window showed, a phone's notification sounds as the phone is set. Never
 * into a deafened call. */
export function liveSoundWanted(
  shown: Shown,
  device: { mobile: boolean; deafened: boolean },
): boolean {
  return (
    (shown === "worker" || shown === "page" || shown === "app") &&
    !device.mobile &&
    !device.deafened
  );
}

/** "Ada ist live in Lounge.", as the chat notice said. */
export function liveText(author: string, voiceName: string): string {
  return `${liveSentence([author], false)} in ${voiceName}.`;
}

/** The system notification's words. With message content hidden (#268)
 * only who, not where. */
export function liveNotificationText(
  hide: boolean,
  notice: { author: string; serverName: string; text: string },
): { title: string; body: string } {
  if (hide) return { title: liveSentence([notice.author], false), body: "" };
  return {
    title: `${notice.author} · ${notice.serverName}`,
    body: notice.text,
  };
}

/** What outlives one listener: the known Lives and the cue's pacing. */
export function createLiveNoticeMemory() {
  return { starts: createLiveStarts(), sound: createNotificationPacing() };
}
export type LiveNoticeMemory = ReturnType<typeof createLiveNoticeMemory>;

export type LiveNoticeDeps = {
  me: string;
  /** Local clock, for the cooldown, the cue's pacing and the notification's
   * id. */
  now: () => number;
  /** Server clock, for mutes and the notification's time. */
  serverNow: () => number;
  /** The server's detail, from the cache or fetched. */
  server: (serverId: string) => Promise<ServerDetail | undefined>;
  setting: (
    serverId: string,
  ) => Pick<NotificationSetting, "level" | "muted" | "muted_until">;
  /** I sit in that voice channel, here or on another device. */
  here: (serverId: string, channelId: string) => boolean;
  device: () => LiveDevice & {
    mobile: boolean;
    deafened: boolean;
    /** Notifications leave the content out (#268). */
    hideContent: boolean;
  };
  /** Still the account and session the listener started for. */
  holds: () => boolean;
  toast: (toast: Omit<MessageToast, "id" | "count" | "at">) => void;
  notify: (
    notification: MessageNotification,
    onClick: () => void,
  ) => Promise<Shown>;
  /** callSounds.ts `playLiveSound`; `messageSound` is its switch. */
  sound: () => void;
  /** Into the voice channel, without joining it. */
  open: (serverId: string, channelId: string) => void;
};

/** One gateway voice frame in, a notice out when it starts a new Live. */
export function createLiveNotices(
  deps: LiveNoticeDeps,
  memory: LiveNoticeMemory = createLiveNoticeMemory(),
): (event: SigEvent) => void {
  /** `heard` and `at`: when the frame came, by the local and the server
   * clock. */
  const announce = async (start: LiveStart, heard: number, at: number) => {
    const server = await deps.server(start.serverId).catch(() => undefined);
    if (!server || !deps.holds()) return;
    const device = deps.device();
    const decision = liveNoticeDecision({
      start,
      me: deps.me,
      here: deps.here(start.serverId, start.channelId),
      server,
      setting: deps.setting(start.serverId),
      now: deps.serverNow(),
      device,
    });
    if (!decision.toast && !decision.desktop) return;
    const member = server.members.find((m) => m.user_id === start.userId);
    const author = member?.name ?? "Jemand";
    const voice = server.channels.find((c) => c.id === start.channelId)!;
    const text = liveText(author, voice.name);
    if (decision.toast)
      deps.toast({
        channelId: start.channelId,
        serverId: start.serverId,
        dm: false,
        channelLabel: server.name,
        author,
        authorAvatar: member?.avatar_url ?? null,
        mentioned: false,
        preview: text,
      });
    if (!decision.desktop) return;
    const shown = await deps.notify(
      {
        ...liveNotificationText(device.hideContent, {
          author,
          serverName: server.name,
          text,
        }),
        // One per voice channel, as one per conversation for messages.
        channelId: start.channelId,
        // Every window hears the start, and only one may announce it. A
        // voice frame carries no id or time to agree on (it is not
        // sequenced), so the windows agree on the cooldown window it came
        // in; two notices of one person there are a cooldown apart, never in
        // one. Taken by the local clock as the frame comes: the windows that
        // share notifications share that clock, while each guesses the
        // server's to the second and may have to fetch the server's detail
        // first. Only frames a moment apart across the edge of that window
        // still get two ids.
        messageId: `live:${start.userId}:${Math.floor(heard / LIVE_NOTICE_COOLDOWN_MS)}`,
        createdAt: new Date(at).toISOString(),
        path: conversationPath(false, start.serverId, start.channelId),
        user: deps.me,
      },
      () => {
        if (deps.holds()) deps.open(start.serverId, start.channelId);
      },
    );
    if (
      liveSoundWanted(shown, deps.device()) &&
      deps.holds() &&
      memory.sound.take("", deps.now())
    )
      deps.sound();
  };

  return (event) => {
    if (event.t === "r") {
      memory.starts.roster(event.s, event.snap ?? [], deps.now());
      return;
    }
    const { u: userId, c: channelId } = event;
    if (!userId || !channelId || userId === deps.me) return;
    if (event.t === "l" || (event.t === "u" && event.k === "s")) {
      memory.starts.end(event.s, channelId, userId, deps.now());
      return;
    }
    // A replay describes who is there already; it starts nothing.
    if (event.t !== "p" || event.k !== "s" || event.replay) return;
    const heard = deps.now();
    if (!memory.starts.start(event.s, channelId, userId, heard)) return;
    if (deps.here(event.s, channelId)) return;
    void announce(
      { serverId: event.s, channelId, userId },
      heard,
      deps.serverNow(),
    );
  };
}
