// Server-wide voice occupancy for the member list (issue 12).
// Join/leave/mute/deafen live on `op: "sig"` — not chat events, no SDK.
// Go Live holders (`live`) are one user per voice channel (issue 14).

import { create } from "zustand";

import type { SigEvent, VoiceEntry } from "../ws/protocol.ts";

export type VoiceFlags = {
  channelId: string;
  muted: boolean;
  deafened: boolean;
};

/** channelId → userId. At most one live track per voice channel. */
export type LiveMap = Record<string, string>;

/** Stable empties for Zustand selectors. A fresh `{}` each call is React 19 #185. */
export const EMPTY_OCCUPANCY: Record<string, VoiceFlags> = Object.freeze({});
export const EMPTY_LIVE: LiveMap = Object.freeze({});

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

export function applyVoiceLeave(serverId: string, userId: string): void {
  useVoiceRoster.setState((state) => {
    const current = state.byServer[serverId];
    const lives = state.live[serverId] ?? {};
    const flags = current?.[userId];
    let nextLive = lives;
    if (flags && lives[flags.channelId] === userId) {
      nextLive = { ...lives };
      delete nextLive[flags.channelId];
    }
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
): void {
  useVoiceRoster.setState((state) => {
    const current = state.live[serverId] ?? {};
    if (current[channelId] === userId) return state;
    return {
      ...state,
      live: {
        ...state.live,
        [serverId]: { ...current, [channelId]: userId },
      },
    };
  });
}

export function applyLiveEnd(
  serverId: string,
  channelId: string,
  userId?: string,
): void {
  useVoiceRoster.setState((state) => {
    const current = state.live[serverId];
    if (!current || !(channelId in current)) return state;
    if (userId && current[channelId] !== userId) return state;
    const next = { ...current };
    delete next[channelId];
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
  const live: LiveMap = {};
  for (const entry of entries) {
    next[entry.u] = {
      channelId: entry.c,
      muted: entry.m === true,
      deafened: entry.d === true,
    };
    if (entry.l) {
      live[entry.c] = entry.u;
    }
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
      applyVoiceLeave(event.s, userId);
      return;
    case "m":
      applyVoiceMute(event.s, userId, event.on === true);
      return;
    case "d":
      applyVoiceDeafen(event.s, userId, event.on === true);
      return;
    case "p":
      if (event.k === "l") applyLiveStart(event.s, channelId, userId);
      return;
    case "u":
      if (event.k === "l") applyLiveEnd(event.s, channelId, userId);
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

export function liveOf(
  live: Record<string, LiveMap>,
  serverId: string,
  channelId: string,
): string | null {
  return live[serverId]?.[channelId] ?? null;
}

export function resetVoiceRoster(): void {
  useVoiceRoster.setState({ byServer: {}, live: {} });
}
