// The message the composer of a channel answers (issue #251). The row's
// "Antworten" button and the composer live in different subtrees.

import { create } from "zustand";

import type { Message } from "./types.ts";

type ReplyTargetState = {
  byChannel: Record<string, Message>;
  set: (channelId: string, message: Message | null) => void;
};

export const useReplyTargets = create<ReplyTargetState>((set) => ({
  byChannel: {},
  set: (channelId, message) =>
    set((state) => {
      const byChannel = { ...state.byChannel };
      if (message) byChannel[channelId] = message;
      else delete byChannel[channelId];
      return { byChannel };
    }),
}));

export function useReplyTarget(channelId: string): Message | undefined {
  return useReplyTargets((state) => state.byChannel[channelId]);
}

export function setReplyTarget(
  channelId: string,
  message: Message | null,
): void {
  useReplyTargets.getState().set(channelId, message);
}

/** An edit or delete of the answered message reaches the composer bar
 *  too: an edit shows there, a delete ends the reply. */
export function refreshReplyTarget(
  channelId: string,
  id: string,
  original: Message | null,
): void {
  const state = useReplyTargets.getState();
  if (state.byChannel[channelId]?.id !== id) return;
  state.set(channelId, original);
}

/** Account switch: a reply target must not follow into another account. */
export function resetReplyTargets(): void {
  useReplyTargets.setState({ byChannel: {} });
}
