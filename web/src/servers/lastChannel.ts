// Which channel the user last had open per server, so clicking a server in
// the rail lands where they left off (falls back to the first text channel).
// Persisted per browser; nothing here is worth a server round trip.

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastChannelState = {
  byServer: Record<string, string>;
  remember: (serverId: string, channelId: string) => void;
  forget: (serverId: string) => void;
};

export const useLastChannel = create<LastChannelState>()(
  persist(
    (set) => ({
      byServer: {},
      remember: (serverId, channelId) =>
        set((state) =>
          state.byServer[serverId] === channelId
            ? state
            : { byServer: { ...state.byServer, [serverId]: channelId } },
        ),
      forget: (serverId) =>
        set((state) => {
          if (!(serverId in state.byServer)) return state;
          const byServer = { ...state.byServer };
          delete byServer[serverId];
          return { byServer };
        }),
    }),
    { name: "gelabber.last-channel" },
  ),
);
