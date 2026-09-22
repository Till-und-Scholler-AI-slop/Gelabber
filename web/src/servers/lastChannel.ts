// Which channel the user last had open per server, so clicking a server in
// the rail lands where they left off (falls back to the first text channel).
// Persisted per browser and per account.

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastChannelState = {
  byUser: Record<string, Record<string, string>>;
  remember: (userId: string, serverId: string, channelId: string) => void;
  forget: (userId: string, serverId: string) => void;
};

export const EMPTY_LAST_CHANNELS: Record<string, string> = {};

export const useLastChannel = create<LastChannelState>()(
  persist(
    (set) => ({
      byUser: {},
      remember: (userId, serverId, channelId) =>
        set((state) => {
          const mine = state.byUser[userId] ?? EMPTY_LAST_CHANNELS;
          if (mine[serverId] === channelId) return state;
          return {
            byUser: {
              ...state.byUser,
              [userId]: { ...mine, [serverId]: channelId },
            },
          };
        }),
      forget: (userId, serverId) =>
        set((state) => {
          const mine = state.byUser[userId];
          if (!mine || !(serverId in mine)) return state;
          const next = { ...mine };
          delete next[serverId];
          return { byUser: { ...state.byUser, [userId]: next } };
        }),
    }),
    {
      name: "gelabber.last-channel",
      version: 1,
      // v0 stored byServer for the whole browser. Drop it rather than hand
      // one account's channel map to the next.
      migrate: () => ({ byUser: {} }),
      partialize: (state) => ({ byUser: state.byUser }),
    },
  ),
);

export function lastChannelsFor(
  byUser: Record<string, Record<string, string>>,
  userId: string | null | undefined,
): Record<string, string> {
  if (!userId) return EMPTY_LAST_CHANNELS;
  return byUser[userId] ?? EMPTY_LAST_CHANNELS;
}
