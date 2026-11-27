// "Zuschauen" from outside the voice channel (LiveHint, the live toast). A
// Live is watched in its voice channel: outside a call this joins it muted
// (GEL-114), with the Lives there watched from the start and the camera off;
// in the same call it watches them and leaves the microphone as it is;
// in another call the room asks before it switches, so only the page opens
// and the button says "Zum Kanal". The caller opens the channel's page.

import { notify } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import { joinVoice, toggleSourceWatch, useVoice } from "./session.ts";

/** The call I am in as `serverId/channelId`, or null: a `useVoice` selector. */
export function currentCall(state: {
  status: string;
  serverId: string | null;
  channelId: string | null;
}): string | null {
  return state.status === "joined"
    ? `${state.serverId}/${state.channelId}`
    : null;
}

/** The button's word for a Live in `serverId`/`channelId`. */
export function watchLabel(
  call: string | null,
  serverId: string,
  channelId: string,
): string {
  return translate(
    voice,
    call !== null && call !== `${serverId}/${channelId}`
      ? "watch.toChannel"
      : "watch.watch",
  );
}

export function joinToWatch(live: {
  serverId: string;
  channelId: string;
  channelName: string | null;
  /** The people live there, besides me. */
  people: string[];
}): void {
  const state = useVoice.getState();
  const call = currentCall(state);
  if (call === null) {
    joinVoice({
      serverId: live.serverId,
      channelId: live.channelId,
      channelName: live.channelName ?? translate(voice, "call.voiceChannel"),
      watch: live.people,
      muted: true,
    });
    notify(translate(voice, "watch.joinedMuted"));
    return;
  }
  if (call !== `${live.serverId}/${live.channelId}`) return;
  for (const userId of live.people)
    if (!state.sourceSubscriptions[userId]?.s) toggleSourceWatch(userId, "s");
}
