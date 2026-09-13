// Server-wide voice occupancy for the member list (issue 12).
// Join/leave/mute/deafen live on `op: "sig"` — not chat events, no SDK.

import { create } from "zustand";

import type { SigEvent, VoiceEntry } from "../ws/protocol.ts";

export type VoiceFlags = {
  channelId: string;
  muted: boolean;
  deafened: boolean;
};

export const VOICE_ICON_SLOT_PX = 32;

type RosterState = {
  byServer: Record<string, Record<string, VoiceFlags>>;
};

export const useVoiceRoster = create<RosterState>(() => ({
  byServer: {},
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
    if (!current || !(userId in current)) return state;
    const next = { ...current };
    delete next[userId];
    return { byServer: { ...state.byServer, [serverId]: next } };
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
      byServer: {
        ...state.byServer,
        [serverId]: { ...current, [userId]: next },
      },
    };
  });
}

export function applyVoiceSnap(serverId: string, entries: VoiceEntry[]): void {
  const next: Record<string, VoiceFlags> = {};
  for (const entry of entries) {
    next[entry.u] = {
      channelId: entry.c,
      muted: entry.m === true,
      deafened: entry.d === true,
    };
  }
  useVoiceRoster.setState((state) => ({
    byServer: { ...state.byServer, [serverId]: next },
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

export function resetVoiceRoster(): void {
  useVoiceRoster.setState({ byServer: {} });
}
