// Ephemeral presence + typing (issue 8). Separate from chat events so a
// status flip never touches the message list or its scroll container.

import { create } from "zustand";

import type { PresenceStatus } from "./protocol.ts";

export const CLIENT_IDLE_MS = 5 * 60 * 1000;
export const PRESENCE_PULSE_MS = 10_000;
export const TYPING_TTL_MS = 6_000;
export const TYPING_REFRESH_MS = 3_000;
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
    return changed || state.now !== now ? { byChannel, now } : state;
  });
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
  if (names.length === 1) return `${names[0]} tippt…`;
  if (names.length === 2) return `${names[0]} und ${names[1]} tippen…`;
  return `${names[0]}, ${names[1]} und ${names.length - 2} weitere tippen…`;
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
    group.sort((a, b) => a.name.localeCompare(b.name, "de"));
  }
  return (
    [
      { group: "o", label: "Online", members: buckets.o },
      { group: "i", label: "Abwesend", members: buckets.i },
      { group: "x", label: "Offline", members: buckets.x },
    ] as const
  ).filter((row) => row.members.length > 0);
}
