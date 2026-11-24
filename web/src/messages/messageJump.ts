// A message to open a channel at, asked for before its page is there
// (the overview's "Geteilt", GEL-69). The channel's message pane opens in
// that message's context, as a search hit does, and then clears it.
import { create } from "zustand";

export type MessageJump = { channelId: string; messageId: string };

/** A request no message pane took up by then is dropped: a later visit to
 *  the channel, maybe by another account, opens as usual. */
export const JUMP_TTL_MS = 10_000;

export const useMessageJump = create<{
  jump: (MessageJump & { at: number }) | null;
}>(() => ({
  jump: null,
}));

export function requestMessageJump(jump: MessageJump): void {
  useMessageJump.setState({ jump: { ...jump, at: Date.now() } });
}

/** The message asked for in this channel, if any. */
export function peekMessageJump(channelId: string): string | null {
  const { jump } = useMessageJump.getState();
  return jump?.channelId === channelId && Date.now() - jump.at < JUMP_TTL_MS
    ? jump.messageId
    : null;
}

/** Done with this channel's request: a later visit opens as usual. */
export function clearMessageJump(channelId: string): void {
  if (useMessageJump.getState().jump?.channelId === channelId)
    useMessageJump.setState({ jump: null });
}
