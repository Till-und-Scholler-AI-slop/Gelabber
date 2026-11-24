// A message to open a channel at, asked for before its page is there
// (the overview's "Geteilt", GEL-69). The channel's message pane opens in
// that message's context, as a search hit does, and then clears it.
import { create } from "zustand";

export type MessageJump = { channelId: string; messageId: string };

export const useMessageJump = create<{ jump: MessageJump | null }>(() => ({
  jump: null,
}));

export function requestMessageJump(jump: MessageJump): void {
  useMessageJump.setState({ jump });
}

/** The message asked for in this channel, if any. */
export function peekMessageJump(channelId: string): string | null {
  const { jump } = useMessageJump.getState();
  return jump?.channelId === channelId ? jump.messageId : null;
}

/** Done with this channel's request: a later visit opens as usual. */
export function clearMessageJump(channelId: string): void {
  if (useMessageJump.getState().jump?.channelId === channelId)
    useMessageJump.setState({ jump: null });
}
