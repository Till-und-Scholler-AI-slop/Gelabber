// In-flight optimistic sends, keyed by channel. Lives outside the query
// cache so an in-flight GET cannot wipe a row the user already sees.

import { create } from "zustand";

import type { Message } from "./types.ts";

/** Stable empty list so a Zustand selector does not return a new `[]` every
 *  time a channel has no in-flight send (that looped MessagePane on mount). */
export const nonePending: Message[] = [];

type PendingState = {
  byChannel: Record<string, Message[]>;
  add: (channelId: string, message: Message) => void;
  remove: (channelId: string, id: string) => void;
  clear: (channelId: string) => void;
};

export const usePendingMessages = create<PendingState>((set) => ({
  byChannel: {},
  add: (channelId, message) =>
    set((state) => ({
      byChannel: {
        ...state.byChannel,
        [channelId]: [...(state.byChannel[channelId] ?? []), message],
      },
    })),
  remove: (channelId, id) =>
    set((state) => ({
      byChannel: {
        ...state.byChannel,
        [channelId]: (state.byChannel[channelId] ?? []).filter(
          (m) => m.id !== id,
        ),
      },
    })),
  clear: (channelId) =>
    set((state) => {
      if (!(channelId in state.byChannel)) return state;
      const next = { ...state.byChannel };
      delete next[channelId];
      return { byChannel: next };
    }),
}));

export function addPending(channelId: string, message: Message): void {
  usePendingMessages.getState().add(channelId, message);
}

export function removePending(channelId: string, id: string): void {
  usePendingMessages.getState().remove(channelId, id);
}
