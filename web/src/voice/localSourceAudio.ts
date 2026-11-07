// The own Live's sound: the browser picker's audio track, switched off while
// it runs.

import { sharesSourceAudio } from "./settings.ts";
import { useVoice, type SourceAudioStatus } from "./voiceStore.ts";
import {
  currentUserId,
  hasLiveTrack,
  hintTrack,
  sendPub,
  setPub,
  voice,
} from "./sessionState.ts";

/** The own Live's sound. */
export function setSourceAudio(status: SourceAudioStatus): void {
  useVoice.setState({ sourceAudio: { s: status } });
}

export function bindSourceAudio(
  stream: MediaStream,
  track: MediaStreamTrack,
): void {
  hintTrack(track, "music");
  track.addEventListener("ended", () => {
    if (voice.screenStream === stream) stopLocalSourceAudio();
  });
}

/** The sound leaves, the video stays: "ended" when the source went away by
 * itself, "off" when the user switched it off. */
function stopLocalSourceAudio(status: SourceAudioStatus = "ended"): void {
  const connection = voice.seat.connection;
  if (connection)
    void connection.closeSource("sa").catch((error) => {
      if (voice.seat.connection === connection) voice.deps?.onError?.(error);
    });
  for (const track of voice.screenStream?.getAudioTracks() ?? []) track.stop();
  const self = currentUserId();
  if (self) setPub(self, "sa", false);
  sendPub("sa", false);
  setSourceAudio(status);
}

/**
 * The stream-sound choice changed. Switching it off takes the sound out of a
 * running Live at once. Switching it on would need the browser's picker
 * again, so it waits for the next Live.
 */
export function applySourceAudioChoice(): void {
  const wanted = sharesSourceAudio();
  const stream = voice.screenStream;
  if (!stream || wanted) return;
  if (hasLiveTrack(stream, "audio")) stopLocalSourceAudio("off");
  else if (useVoice.getState().sourceAudio.s !== "off") setSourceAudio("off");
}
