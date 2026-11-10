// Ephemeral presence + typing (issue 8). Separate from chat events so a
// status flip never touches the message list or its scroll container.

import { create } from "zustand";

import type { PresenceStatus } from "./protocol.ts";
import { translate } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { compareNames } from "../i18n/format.ts";

export const CLIENT_IDLE_MS = 5 * 60 * 1000;
export const PRESENCE_PULSE_MS = 10_000;
/** Real user input. `visibilitychange` is not in this list. */
export const PRESENCE_ACTIVITY_EVENTS = [
  "pointerdown",
  "keydown",
  "mousemove",
] as const;

export function isPresenceActivityEvent(type: string): boolean {
  return (PRESENCE_ACTIVITY_EVENTS as readonly string[]).includes(type);
}
export const TYPING_TTL_MS = 6_000;
export const TYPING_REFRESH_MS = 3_000;
/** No keystroke for this long ends "tippt…" though the caret stays put. */
export const TYPING_IDLE_MS = 6_000;
/** Reserved strip under the message pane — typing must not relayout chat. */
export const TYPING_BAR_PX = 28;

export type PresenceMap = Record<string, PresenceStatus>;

type PresenceState = {
  byServer: Record<string, PresenceMap>;
};

export const usePresenceStore = create<PresenceState>(() => ({
  byServer: {},
}));

export function applyPresence(
  serverId: string,
  userId: string,
  status: PresenceStatus,
): void {
  usePresenceStore.setState((state) => {
    const current = state.byServer[serverId] ?? {};
    if (status === "x") {
      if (!(userId in current)) return state;
      const next = { ...current };
      delete next[userId];
      return { byServer: { ...state.byServer, [serverId]: next } };
    }
    if (current[userId] === status) return state;
    return {
      byServer: {
        ...state.byServer,
        [serverId]: { ...current, [userId]: status },
      },
    };
  });
}

export function applyPresenceSnap(
  serverId: string,
  entries: { u: string; st: PresenceStatus }[],
): void {
  const next: PresenceMap = {};
  for (const entry of entries) {
    if (entry.st !== "x") next[entry.u] = entry.st;
  }
  usePresenceStore.setState((state) => ({
    byServer: { ...state.byServer, [serverId]: next },
  }));
}

export function presenceOf(
  byServer: Record<string, PresenceMap>,
  serverId: string,
  userId: string,
): PresenceStatus {
  return byServer[serverId]?.[userId] ?? "x";
}

export type TypingMap = Record<string, number>;

type TypingState = {
  byChannel: Record<string, TypingMap>;
  now: number;
};

export const useTypingStore = create<TypingState>(() => ({
  byChannel: {},
  now: 0,
}));

export function applyTyping(
  channelId: string,
  userId: string,
  on: boolean,
  now: number,
  ttl = TYPING_TTL_MS,
): void {
  useTypingStore.setState((state) => {
    const current = state.byChannel[channelId] ?? {};
    if (!on) {
      if (!(userId in current)) return state;
      const next = { ...current };
      delete next[userId];
      return { byChannel: { ...state.byChannel, [channelId]: next }, now };
    }
    return {
      byChannel: {
        ...state.byChannel,
        [channelId]: { ...current, [userId]: now + ttl },
      },
      now,
    };
  });
}

export function pruneTyping(now: number): void {
  useTypingStore.setState((state) => {
    let changed = false;
    const byChannel: Record<string, TypingMap> = {};
    for (const [channelId, users] of Object.entries(state.byChannel)) {
      const next: TypingMap = {};
      for (const [userId, until] of Object.entries(users)) {
        if (until > now) next[userId] = until;
        else changed = true;
      }
      byChannel[channelId] = next;
    }
    // Only an expiry is news: a fresh state per tick re-rendered every reader.
    return changed ? { byChannel, now } : state;
  });
}

/**
 * Announces typing for one composer: on at the first keystroke, refreshed
 * while keys keep coming, off when the field empties, on `stop`, or after
 * `TYPING_IDLE_MS` without a keystroke.
 */
export function createTypingSender(
  send: (serverId: string, channelId: string, on: boolean) => void,
  now: () => number = Date.now,
): {
  input: (serverId: string, channelId: string, value: string) => void;
  stop: () => void;
} {
  let sent: { s: string; c: string } | null = null;
  let refresh: ReturnType<typeof setInterval> | null = null;
  let lastInput = 0;

  const stop = () => {
    if (refresh) {
      clearInterval(refresh);
      refresh = null;
    }
    const target = sent;
    sent = null;
    if (target) send(target.s, target.c, false);
  };

  const input = (serverId: string, channelId: string, value: string) => {
    if (value.length === 0) {
      stop();
      return;
    }
    lastInput = now();
    if (sent) return;
    const target = { s: serverId, c: channelId };
    sent = target;
    send(target.s, target.c, true);
    refresh = setInterval(() => {
      if (now() - lastInput >= TYPING_IDLE_MS) stop();
      else send(target.s, target.c, true);
    }, TYPING_REFRESH_MS);
  };

  return { input, stop };
}

export function typingUserIds(
  byChannel: Record<string, TypingMap>,
  channelId: string,
  now: number,
  exceptUserId?: string,
): string[] {
  const users = byChannel[channelId];
  if (!users) return [];
  return Object.entries(users)
    .filter(([userId, until]) => until > now && userId !== exceptUserId)
    .map(([userId]) => userId);
}

export function formatTyping(names: string[]): string {
  if (names.length === 0) return "";
  if (names.length === 1)
    return translate(shell, "typing.one", { name: names[0] });
  if (names.length === 2)
    return translate(shell, "typing.two", {
      first: names[0],
      second: names[1],
    });
  return translate(shell, "typing.more", {
    first: names[0],
    second: names[1],
    count: names.length - 2,
  });
}

export function resetLiveStores(): void {
  usePresenceStore.setState({ byServer: {} });
  useTypingStore.setState({ byChannel: {}, now: 0 });
}

export type PresenceGroup = "o" | "i" | "x";

export function groupMembers<T extends { user_id: string; name: string }>(
  members: T[],
  statusOf: (userId: string) => PresenceStatus,
): { group: PresenceGroup; label: string; members: T[] }[] {
  const buckets: Record<PresenceGroup, T[]> = { o: [], i: [], x: [] };
  for (const member of members) {
    buckets[statusOf(member.user_id)].push(member);
  }
  for (const group of Object.values(buckets)) {
    group.sort((a, b) => compareNames(a.name, b.name));
  }
  return (
    [
      { group: "o", label: translate(shell, "presence.o"), members: buckets.o },
      { group: "i", label: translate(shell, "presence.i"), members: buckets.i },
      { group: "x", label: translate(shell, "presence.x"), members: buckets.x },
    ] as const
  ).filter((row) => row.members.length > 0);
}
