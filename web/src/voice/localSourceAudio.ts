// The own Live's sound: the browser picker's audio track, or the desktop
// app's capture of an application, switched on and off while it runs.

import { MediaError } from "./media.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { captureAppAudio } from "./native/capture.ts";
import { sharesSourceAudio, useMediaSettings } from "./settings.ts";
import {
  useVoice,
  type SourceAudioNote,
  type SourceAudioStatus,
} from "./voiceStore.ts";
import {
  applySendBitrate,
  captureEpoch,
  currentUserId,
  hasLiveTrack,
  hintTrack,
  logVoice,
  sendPub,
  setPub,
  voice,
} from "./sessionState.ts";

/** The own Live's sound. */
export function setSourceAudio(
  status: SourceAudioStatus,
  note: SourceAudioNote | null = null,
): void {
  useVoice.setState({
    sourceAudio: { s: status },
    sourceAudioNote: { s: note },
  });
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
function stopLocalSourceAudio(
  status: SourceAudioStatus = "ended",
  note: SourceAudioNote | null = null,
): void {
  const connection = voice.seat.connection;
  if (connection)
    void connection.closeSource("sa").catch((error) => {
      if (voice.seat.connection === connection) voice.deps?.onError?.(error);
    });
  for (const track of voice.screenStream?.getAudioTracks() ?? []) track.stop();
  const self = currentUserId();
  if (self) setPub(self, "sa", false);
  sendPub("sa", false);
  setSourceAudio(status, note);
}

/** A chosen application may pause between two songs; only one that stays
 * without a playback stream for this many seconds is reported. */
const SILENT_SECONDS = 3;

/** Desktop app: the chosen application's sound for `stream`'s Live, or the
 * core's reason why there is none. */
export async function captureNativeSourceAudio(
  stream: MediaStream,
): Promise<{ track: MediaStreamTrack } | { failed: string }> {
  const app = useMediaSettings.getState().sourceAudioApp;
  let silent = 0;
  // "Every application" with nothing playing is ordinary silence.
  const heard = (streams: number) => {
    silent = streams > 0 ? 0 : silent + 1;
    noteSilentSource(stream, silent >= SILENT_SECONDS ? app : null);
  };
  try {
    return { track: await captureAppAudio(app, app ? heard : undefined) };
  } catch (error) {
    const failed = error instanceof Error ? error.message : String(error);
    logVoice("warn", "source-audio", { detail: failed });
    return { failed };
  }
}

function noteSilentSource(stream: MediaStream, app: string | null): void {
  const state = useVoice.getState();
  if (voice.screenStream !== stream) return;
  if (state.sourceAudio.s !== "sharing") return;
  if ((state.sourceAudioNote.s?.silent ?? null) === app) return;
  setSourceAudio("sharing", app ? { silent: app } : null);
}

/**
 * The stream-sound choice changed. Switching it off takes the sound out of a
 * running Live at once. Switching it on, or choosing another application,
 * needs a new capture: the desktop app starts one next to the running video,
 * a browser could only reopen its picker, so there it waits for the next
 * Live.
 */
export function applySourceAudioChoice(appChanged: boolean): void {
  const wanted = sharesSourceAudio();
  const desktop = isDesktopApp();
  const stream = voice.screenStream;
  if (!stream) return;
  voice.sourceAudioChange += 1;
  const running = hasLiveTrack(stream, "audio");
  if (running && (!wanted || (desktop && appChanged)))
    stopLocalSourceAudio("off");
  else if (!wanted && useVoice.getState().sourceAudio.s !== "off")
    setSourceAudio("off");
  if (wanted && desktop && (!running || appChanged))
    void addLocalSourceAudio(stream);
}

/** Desktop app: application sound joins a Live that already runs. */
async function addLocalSourceAudio(stream: MediaStream): Promise<void> {
  const change = voice.sourceAudioChange;
  const current = () =>
    voice.sourceAudioChange === change &&
    voice.screenStream === stream &&
    hasLiveTrack(stream, "video");
  const captured = await captureNativeSourceAudio(stream);
  if (!current()) {
    if ("track" in captured) captured.track.stop();
    return;
  }
  if ("failed" in captured) {
    setSourceAudio("unavailable", captured);
    return;
  }
  const track = captured.track;
  // Sound that was switched off before leaves its ended track behind.
  for (const old of stream.getAudioTracks())
    if (old.readyState === "ended") stream.removeTrack(old);
  stream.addTrack(track);
  bindSourceAudio(stream, track);
  setSourceAudio("sharing");
  // Before the video is on the wire, `publishLocal` sends both.
  const connection = voice.seat.connection;
  const parent = connection?.sender("s");
  if (!connection || !parent) return;
  const mine = () =>
    current() &&
    voice.seat.connection === connection &&
    track.readyState === "live";
  try {
    const sender = await connection.publish({
      kind: "sa",
      track,
      streamId: stream.id,
      epoch: captureEpoch(stream),
      parent: parent.producerId,
    });
    if (!mine()) {
      if (connection.sender("sa") === sender)
        await connection.closeSource("sa", sender.producerId).catch(() => {});
      return;
    }
    const self = currentUserId();
    if (self) setPub(self, "sa", true);
    sendPub("sa", true);
    await applySendBitrate();
  } catch (error) {
    // The video is untouched: only the sound did not make it.
    if (!mine()) return;
    // The media socket went away under it. The sound stays in the Live, and
    // the seat's next connection publishes it with the video.
    if (
      !voice.seat.isOpen() ||
      (error instanceof MediaError && error.code === "connection_closed")
    )
      return;
    const failed = error instanceof Error ? error.message : String(error);
    logVoice("warn", "source-audio", { detail: failed });
    stopLocalSourceAudio("unavailable", { failed });
  }
}
