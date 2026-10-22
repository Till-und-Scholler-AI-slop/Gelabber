// Server-wide voice occupancy for the member list (issue 12).
// Join/leave/mute/deafen live on `op: "sig"` — not chat events, no SDK.
// Who is live (`live`): several people per voice channel, each with a
// screen (`s`).

import { create } from "zustand";

import type { SigEvent, VoiceEntry } from "../ws/protocol.ts";

export type VoiceFlags = {
  channelId: string;
  muted: boolean;
  deafened: boolean;
};

export type LiveKind = "s";
/** What one person is live with. */
export type LiveKinds = Partial<Record<LiveKind, true>>;
/** userId → what they are live with, in the order they went live. */
export type LivePeople = Record<string, LiveKinds>;
/** channelId → who is live there. */
export type LiveMap = Record<string, LivePeople>;

/** Stable empties for Zustand selectors. A fresh `{}` each call is React 19 #185. */
export const EMPTY_OCCUPANCY: Record<string, VoiceFlags> = Object.freeze({});
export const EMPTY_LIVE: LiveMap = Object.freeze({});
export const EMPTY_LIVE_PEOPLE: LivePeople = Object.freeze({});

export const VOICE_ICON_SLOT_PX = 32;

type RosterState = {
  byServer: Record<string, Record<string, VoiceFlags>>;
  live: Record<string, LiveMap>;
};

export const useVoiceRoster = create<RosterState>(() => ({
  byServer: {},
  live: {},
}));

export function applyVoiceJoin(
  serverId: string,
  userId: string,
  channelId: string,
  flags?: { muted?: boolean; deafened?: boolean },
): void {
  useVoiceRoster.setState((state) => {
    const current = state.byServer[serverId] ?? {};
    const prev = current[userId];
    const next: VoiceFlags = {
      channelId,
      muted: flags?.muted ?? prev?.muted ?? false,
      deafened: flags?.deafened ?? prev?.deafened ?? false,
    };
    if (
      prev &&
      prev.channelId === next.channelId &&
      prev.muted === next.muted &&
      prev.deafened === next.deafened
    ) {
      return state;
    }
    return {
      ...state,
      byServer: {
        ...state.byServer,
        [serverId]: { ...current, [userId]: next },
      },
    };
  });
}

export function applyVoiceLeave(
  serverId: string,
  userId: string,
  channelId?: string,
): void {
  useVoiceRoster.setState((state) => {
    const current = state.byServer[serverId];
    const lives = state.live[serverId] ?? EMPTY_LIVE;
    const flags = current?.[userId];
    // The leave of a channel they are no longer shown in (another device's
    // seat, or a move that got here first) ends only what was there.
    const elsewhere =
      channelId !== undefined &&
      flags !== undefined &&
      flags.channelId !== channelId;
    // Nobody is live in a channel they left.
    let nextLive = lives;
    for (const channel of new Set([
      channelId,
      elsewhere ? undefined : flags?.channelId,
    ])) {
      if (channel && nextLive[channel]?.[userId])
        nextLive = withoutLive(nextLive, channel, userId);
    }
    if (elsewhere)
      return nextLive === lives
        ? state
        : { live: { ...state.live, [serverId]: nextLive } };
    if ((!current || !(userId in current)) && nextLive === lives) {
      return state;
    }
    const nextOccupancy = current ? { ...current } : {};
    delete nextOccupancy[userId];
    return {
      byServer: { ...state.byServer, [serverId]: nextOccupancy },
      live: { ...state.live, [serverId]: nextLive },
    };
  });
}

/** `live` without `userId` in `channelId`, or only without its `kind`. */
function withoutLive(
  live: LiveMap,
  channelId: string,
  userId: string,
  kind?: LiveKind,
): LiveMap {
  const people = live[channelId];
  const kinds = people?.[userId];
  if (!kinds || (kind && !kinds[kind])) return live;
  const rest: LiveKinds = { ...kinds };
  if (kind) delete rest[kind];
  const nextPeople = { ...people };
  if (!kind || !rest.s) delete nextPeople[userId];
  else nextPeople[userId] = rest;
  const next = { ...live };
  if (Object.keys(nextPeople).length === 0) delete next[channelId];
  else next[channelId] = nextPeople;
  return next;
}

export function applyVoiceMute(
  serverId: string,
  userId: string,
  muted: boolean,
): void {
  patchFlags(serverId, userId, { muted });
}

export function applyVoiceDeafen(
  serverId: string,
  userId: string,
  deafened: boolean,
): void {
  patchFlags(serverId, userId, {
    deafened,
    ...(deafened ? { muted: true } : {}),
  });
}

export function applyLiveStart(
  serverId: string,
  channelId: string,
  userId: string,
  kind: LiveKind,
): void {
  useVoiceRoster.setState((state) => {
    const current = state.live[serverId] ?? EMPTY_LIVE;
    const people = current[channelId] ?? EMPTY_LIVE_PEOPLE;
    if (people[userId]?.[kind]) return state;
    return {
      ...state,
      live: {
        ...state.live,
        [serverId]: {
          ...current,
          [channelId]: {
            ...people,
            [userId]: { ...people[userId], [kind]: true },
          },
        },
      },
    };
  });
}

/** `userId` stops being live in `channelId`: with `kind`, or at all. */
export function applyLiveEnd(
  serverId: string,
  channelId: string,
  userId: string,
  kind?: LiveKind,
): void {
  useVoiceRoster.setState((state) => {
    const current = state.live[serverId];
    if (!current) return state;
    const next = withoutLive(current, channelId, userId, kind);
    if (next === current) return state;
    return { ...state, live: { ...state.live, [serverId]: next } };
  });
}

function patchFlags(
  serverId: string,
  userId: string,
  patch: Partial<Pick<VoiceFlags, "muted" | "deafened">>,
): void {
  useVoiceRoster.setState((state) => {
    const current = state.byServer[serverId] ?? {};
    const prev = current[userId];
    if (!prev) return state;
    const next = { ...prev, ...patch };
    if (prev.muted === next.muted && prev.deafened === next.deafened) {
      return state;
    }
    return {
      ...state,
      byServer: {
        ...state.byServer,
        [serverId]: { ...current, [userId]: next },
      },
    };
  });
}

export function applyVoiceSnap(serverId: string, entries: VoiceEntry[]): void {
  const next: Record<string, VoiceFlags> = {};
  const live: Record<string, LivePeople> = {};
  for (const entry of entries) {
    next[entry.u] = {
      channelId: entry.c,
      muted: entry.m === true,
      deafened: entry.d === true,
    };
    if (entry.s) live[entry.c] = { ...live[entry.c], [entry.u]: { s: true } };
  }
  useVoiceRoster.setState((state) => ({
    byServer: { ...state.byServer, [serverId]: next },
    live: { ...state.live, [serverId]: live },
  }));
}

export function applyVoiceSig(event: SigEvent): void {
  if (event.t === "r") {
    applyVoiceSnap(event.s, event.snap ?? []);
    return;
  }
  const userId = event.u;
  const channelId = event.c;
  if (!userId || !channelId) return;
  switch (event.t) {
    case "j":
      applyVoiceJoin(event.s, userId, channelId, {
        muted: event.m,
        deafened: event.d,
      });
      return;
    case "l":
      applyVoiceLeave(event.s, userId, channelId);
      return;
    case "m":
      applyVoiceMute(event.s, userId, event.on === true);
      return;
    case "d":
      applyVoiceDeafen(event.s, userId, event.on === true);
      return;
    case "p":
      if (event.k === "s") applyLiveStart(event.s, channelId, userId, event.k);
      return;
    case "u":
      if (event.k === "s") applyLiveEnd(event.s, channelId, userId, event.k);
      return;
    default:
      return;
  }
}

export function voiceOf(
  byServer: Record<string, Record<string, VoiceFlags>>,
  serverId: string,
  userId: string,
): VoiceFlags | null {
  return byServer[serverId]?.[userId] ?? null;
}

/** Who is live in `channelId`, in the order they went live. */
export function liveIn(
  live: Record<string, LiveMap>,
  serverId: string,
  channelId: string,
): string[] {
  return Object.keys(live[serverId]?.[channelId] ?? EMPTY_LIVE_PEOPLE);
}

/** Whether `userId` is live anywhere in `serverId`. */
export function isLiveIn(live: LiveMap, userId: string): boolean {
  return Object.values(live).some((people) => Boolean(people[userId]));
}

export function resetVoiceRoster(): void {
  useVoiceRoster.setState({ byServer: {}, live: {} });
}
