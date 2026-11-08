// "Zuschauen" from outside the voice channel (LiveHint, the live toast). A
// Live is watched in its voice channel: outside a call this joins it with
// the Lives there watched from the start; in the same call it watches them;
// in another call the room asks before it switches, so only the page opens.
// The caller opens the channel's page.

import { joinVoice, toggleSourceWatch, useVoice } from "./session.ts";

export function joinToWatch(live: {
  serverId: string;
  channelId: string;
  channelName: string | null;
  /** The people live there, besides me. */
  people: string[];
}): void {
  const state = useVoice.getState();
  if (state.status !== "joined") {
    joinVoice({
      serverId: live.serverId,
      channelId: live.channelId,
      channelName: live.channelName ?? "Sprachkanal",
      watch: live.people,
    });
    return;
  }
  if (state.serverId !== live.serverId || state.channelId !== live.channelId)
    return;
  for (const userId of live.people)
    if (!state.sourceSubscriptions[userId]?.s) toggleSourceWatch(userId, "s");
}
