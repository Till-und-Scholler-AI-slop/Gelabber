// Last opened 1:1 DM, so the Home tile lands where the user left off.
// Persisted per browser; nothing here is worth a server round trip.

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastDmState = {
  channelId: string | null;
  remember: (channelId: string) => void;
  forget: (channelId?: string) => void;
};

export const useLastDm = create<LastDmState>()(
  persist(
    (set, get) => ({
      channelId: null,
      remember: (channelId) =>
        get().channelId === channelId ? undefined : set({ channelId }),
      forget: (channelId) => {
        if (channelId && get().channelId !== channelId) return;
        set({ channelId: null });
      },
    }),
    { name: "gelabber.last-dm" },
  ),
);
