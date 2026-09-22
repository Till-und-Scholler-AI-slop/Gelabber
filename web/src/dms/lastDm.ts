// Last opened 1:1 DM, so the Home tile lands where the user left off.
// Persisted per browser and per account. Another login on this browser
// does not inherit the previous account's channel id.

import { create } from "zustand";
import { persist } from "zustand/middleware";

type LastDmState = {
  byUser: Record<string, string>;
  remember: (userId: string, channelId: string) => void;
  forget: (userId: string, channelId?: string) => void;
};

export const useLastDm = create<LastDmState>()(
  persist(
    (set, get) => ({
      byUser: {},
      remember: (userId, channelId) => {
        if (get().byUser[userId] === channelId) return;
        set((state) => ({
          byUser: { ...state.byUser, [userId]: channelId },
        }));
      },
      forget: (userId, channelId) => {
        const current = get().byUser[userId];
        if (!current) return;
        if (channelId && current !== channelId) return;
        set((state) => {
          const byUser = { ...state.byUser };
          delete byUser[userId];
          return { byUser };
        });
      },
    }),
    {
      name: "gelabber.last-dm",
      version: 1,
      // v0 stored one channelId for the whole browser. That id belonged to
      // whoever was signed in last; do not assign it to a new account.
      migrate: () => ({ byUser: {} }),
      partialize: (state) => ({ byUser: state.byUser }),
    },
  ),
);

export function lastDmId(
  byUser: Record<string, string>,
  userId: string | null | undefined,
): string | null {
  if (!userId) return null;
  return byUser[userId] ?? null;
}
