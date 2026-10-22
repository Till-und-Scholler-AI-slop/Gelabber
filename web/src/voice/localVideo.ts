// The own camera and Live (a screen): capture with its stream profile,
// the local preview, publishing on the seat's connection, and stopping.

import type { TrackKind } from "../ws/protocol.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { loadNativeFeatures } from "./native/features.ts";
import type { MediaConnection } from "./mediasoupConnection.ts";
import { noteDiagnosticEvent } from "./diagnostics.ts";
import {
  type StreamKind,
  SOURCE_AUDIO_CONSTRAINTS,
  isOverconstrainedError,
  noteStreamProfileApply,
  sharesSourceAudio,
  useMediaSettings,
  videoConstraintLadder,
  videoConstraintsFor,
} from "./settings.ts";
import { useVoice } from "./voiceStore.ts";
import {
  bindSourceAudio,
  captureNativeSourceAudio,
  setSourceAudio,
} from "./localSourceAudio.ts";
import {
  applySendBitrate,
  captureEpoch,
  currentUserId,
  defaultGetDisplayMedia,
  defaultGetUserMedia,
  hasLiveTrack,
  hintTrack,
  type LocalVideo,
  logVoice,
  publishingConnection,
  reportStreamOnce,
  type RtpSender,
  sendPub,
  setPub,
  stopTracks,
  voice,
} from "./sessionState.ts";

function noteStream(kind: LocalVideo, started: boolean): void {
  const state = useVoice.getState();
  noteDiagnosticEvent({
    kind: started ? "stream-start" : "stream-stop",
    connection: "voice",
    detail: kind === "v" ? "camera" : "screen",
    streaming: state.camera || state.sharing,
  });
}

function liveVideoTracks(kind: StreamKind): MediaStreamTrack[] {
  const streams =
    kind === "camera" ? [voice.cameraStream] : [voice.screenStream];
  const tracks: MediaStreamTrack[] = [];
  for (const stream of streams) {
    const track = stream?.getVideoTracks()[0];
    if (!track) continue;
    const state = (track as MediaStreamTrack & { readyState?: string })
      .readyState;
    if (state === "ended") continue;
    tracks.push(track);
  }
  return tracks;
}

function applyTrackConstraints(
  track: MediaStreamTrack,
  constraints: MediaTrackConstraints,
): Promise<void> | null {
  const apply = (
    track as MediaStreamTrack & {
      applyConstraints?: (next: MediaTrackConstraints) => Promise<void>;
    }
  ).applyConstraints;
  if (!apply) return null;
  return apply.call(track, constraints);
}

/**
 * Bitrate and FPS caps update on the sender immediately. Resolution uses
 * applyConstraints when the browser allows it; otherwise the next capture
 * picks up the profile and the settings form says so.
 */
export async function applyStreamProfile(kind: StreamKind): Promise<void> {
  const epoch =
    kind === "camera" ? ++voice.cameraProfileEpoch : ++voice.screenProfileEpoch;
  const current = () =>
    (kind === "camera"
      ? voice.cameraProfileEpoch
      : voice.screenProfileEpoch) === epoch;
  const pc = voice.seat.connection;
  if (pc) {
    try {
      await voice.videoBudget.enqueue(pc);
    } catch {
      // sender caps are best-effort
    }
  }
  if (!current()) return;
  const tracks = liveVideoTracks(kind);
  if (tracks.length === 0 || useVoice.getState().status !== "joined") {
    noteStreamProfileApply(kind, "idle");
    return;
  }
  const selected =
    kind === "camera"
      ? useMediaSettings.getState().cameraProfile
      : useMediaSettings.getState().screenProfile;
  const constraints = videoConstraintsFor(kind, selected);
  let allApplied = true;
  for (const track of tracks) {
    const pending = applyTrackConstraints(track, constraints);
    if (!pending) {
      allApplied = false;
      continue;
    }
    try {
      await pending;
    } catch {
      allApplied = false;
    }
    if (!current()) return;
  }
  noteStreamProfileApply(kind, allApplied ? "live" : "next");
}

const STREAM_PROFILE_FALLBACK =
  "Dieses Streamprofil wird nicht unterstützt. Es läuft eine sicherere Auflösung.";

async function captureVideo(kind: LocalVideo): Promise<MediaStream> {
  const streamKind: StreamKind = kind === "v" ? "camera" : "screen";
  const settings = useMediaSettings.getState();
  const profile =
    streamKind === "camera" ? settings.cameraProfile : settings.screenProfile;
  const ladder = videoConstraintLadder(
    streamKind,
    profile,
    streamKind === "camera" ? settings.videoInputId : "",
  );
  const getMedia =
    kind === "v"
      ? (voice.deps?.getUserMedia ?? defaultGetUserMedia)
      : (voice.deps?.getDisplayMedia ?? defaultGetDisplayMedia);
  if (kind !== "v") {
    const desktop = isDesktopApp();
    // The app's features decide the default. It answered while the page
    // loaded; a browser must reach its picker within the click.
    if (desktop) await loadNativeFeatures();
    // One browser picker supplies both tracks; profile fallback never reopens it.
    const asked = !desktop && sharesSourceAudio();
    const stream = await getMedia({
      audio: asked ? { ...SOURCE_AUDIO_CONSTRAINTS } : false,
      video: ladder[0] ?? true,
    });
    voice.pendingDisplayStreams.add(stream);
    if (!asked) stream.getAudioTracks().forEach((track) => track.stop());
    // The desktop's picker gives no sound; the app captures it by itself,
    // by the choice as it is now that the picker has closed.
    const sound: { asked: boolean; failed?: string } = {
      asked: desktop ? sharesSourceAudio() : asked,
    };
    voice.displaySound.set(stream, sound);
    if (desktop && sound.asked) {
      const captured = await captureNativeSourceAudio(stream);
      if ("failed" in captured) sound.failed = captured.failed;
      // Stopped while the sound was being captured: nothing may outlive it.
      else if (!hasLiveTrack(stream, "video")) captured.track.stop();
      else stream.addTrack(captured.track);
    }
    const track = stream.getVideoTracks()[0];
    if (track?.applyConstraints) {
      for (let index = 0; index < ladder.length; index += 1) {
        try {
          await track.applyConstraints(ladder[index]);
          if (index > 0)
            voice.deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
          break;
        } catch (error) {
          if (track.readyState === "ended") break;
          if (!isOverconstrainedError(error) || index + 1 === ladder.length) {
            // Capture already succeeded. Keep the browser's usable source.
            voice.deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
            break;
          }
        }
      }
    }
    return stream;
  }
  let lastError: unknown;
  for (let index = 0; index < ladder.length; index += 1) {
    try {
      const stream = await getMedia({
        audio: false,
        video: ladder[index] ?? true,
      });
      if (index > 0) voice.deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
      return stream;
    } catch (error) {
      lastError = error;
      const more = index + 1 < ladder.length;
      if (!more || !isOverconstrainedError(error)) throw error;
    }
  }
  throw lastError instanceof Error ? lastError : new Error("capture failed");
}

function enqueueCameraCommit(job: () => Promise<void>): Promise<void> {
  const run = voice.cameraCommitChain.then(job, job);
  voice.cameraCommitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function cameraCurrent(
  pc: MediaConnection,
  session: number,
  epoch: number,
): boolean {
  return (
    voice.seat.connection === pc &&
    voice.seat.generation === session &&
    voice.cameraEpoch === epoch &&
    useVoice.getState().status === "joined" &&
    useVoice.getState().camera
  );
}

export async function refreshCamera(): Promise<void> {
  const pc = voice.seat.connection;
  if (!pc || !useVoice.getState().camera) return;
  const session = voice.seat.generation;
  const epoch = ++voice.cameraEpoch;
  let stream: MediaStream;
  try {
    stream = await captureVideo("v");
  } catch (error) {
    if (isOverconstrainedError(error) && cameraCurrent(pc, session, epoch)) {
      voice.deps?.onError?.(
        new Error("Die Kamera unterstützt das Streamprofil nicht."),
      );
    }
    return;
  }
  if (!cameraCurrent(pc, session, epoch)) {
    stopTracks(stream);
    return;
  }
  const track = stream.getVideoTracks()[0];
  if (!track) {
    stopTracks(stream);
    return;
  }
  track.enabled = false;
  voice.pendingCameraStreams.add(stream);
  await enqueueCameraCommit(async () => {
    if (!cameraCurrent(pc, session, epoch)) {
      stopTracks(stream);
      voice.pendingCameraStreams.delete(stream);
      return;
    }
    bindEnded(stream, "v");
    const sender = cameraSender();
    const previous = sender?.track ?? null;
    try {
      await pc.publish({
        kind: "v",
        track,
        streamId: stream.id,
        epoch: captureEpoch(stream),
      });
    } catch (error) {
      stopTracks(stream);
      voice.pendingCameraStreams.delete(stream);
      if (cameraCurrent(pc, session, epoch)) {
        voice.deps?.onError?.(
          error instanceof Error
            ? error
            : new Error(
                "Kamerawechsel fehlgeschlagen; bisherige Kamera bleibt aktiv",
              ),
        );
      }
      return;
    }
    if (!cameraCurrent(pc, session, epoch)) {
      if (
        voice.seat.generation === session &&
        voice.seat.connection === pc &&
        useVoice.getState().camera
      ) {
        try {
          if (previous && voice.cameraStream)
            await pc.publish({
              kind: "v",
              track: previous,
              streamId: voice.cameraStream.id,
              epoch: captureEpoch(voice.cameraStream),
            });
          else await pc.closeSource("v");
        } catch {
          // seat.connection may already be tearing down
        }
      }
      stopTracks(stream);
      voice.pendingCameraStreams.delete(stream);
      return;
    }
    const old = voice.cameraStream;
    voice.cameraStream = stream;
    track.enabled = true;
    useVoice.setState({ camera: true, localCamera: stream });
    voice.pendingCameraStreams.delete(stream);
    stopTracks(old);
    await voice.videoBudget.enqueue(pc);
  });
}

function cameraSender(): RtpSender | undefined {
  const cam = voice.cameraStream?.getVideoTracks()[0];
  if (!cam) return undefined;
  return voice.seat.connection
    ?.senders()
    .find((sender) => sender.track === cam);
}

function endedListener(kind: LocalVideo): () => void {
  return () => {
    const state = useVoice.getState();
    if (kind === "v" && state.camera) stopLocalVideo("v");
    if (kind === "s" && state.sharing) stopLocalVideo("s");
  };
}

function bindEnded(stream: MediaStream, kind: LocalVideo): void {
  const onEnded = endedListener(kind);
  for (const track of stream.getVideoTracks()) {
    track.addEventListener("ended", onEnded);
  }
}

function videoEpoch(kind: LocalVideo): number {
  return kind === "v" ? voice.cameraEpoch : voice.screenEpoch;
}

export function bumpVideoEpoch(kind: LocalVideo): number {
  return kind === "v" ? ++voice.cameraEpoch : ++voice.screenEpoch;
}

/** Stopping bumps the epoch, so an abandoned start is not open. A rebuild of
 * the seat's connection leaves an open start its epoch. */
export function captureOpen(kind: LocalVideo): boolean {
  return voice.openCaptures.get(kind) === videoEpoch(kind);
}

/** A camera that is missing, busy or broken: `NotFoundError` and
 * `NotReadableError` of getUserMedia, the latter also what the desktop app's
 * capture reports. A refused permission or a closed prompt is the user's
 * own answer and gets no message. */
function cameraUnavailable(error: unknown): boolean {
  const name = (error as { name?: unknown } | null)?.name;
  return name === "NotFoundError" || name === "NotReadableError";
}

/** A display capture the user called off: a browser's picker and the
 * desktop's own both answer that with `NotAllowedError`. Everything else is
 * a capture that could not start. */
function displayCancelled(error: unknown): boolean {
  return (error as { name?: unknown } | null)?.name === "NotAllowedError";
}

/** What the user is told then. The desktop app's core refuses with its own
 * reason as a plain string ("screen capture needs a Wayland session with
 * PipeWire"); nothing else in the app would show it. */
function displayFailure(error: unknown): Error {
  const reason = typeof error === "string" ? error.trim() : "";
  return new Error(
    `Die Bildschirmaufnahme konnte nicht gestartet werden${reason ? ` (${reason})` : ""}.`,
  );
}

export async function startLocalVideo(kind: LocalVideo): Promise<void> {
  const epoch = bumpVideoEpoch(kind);
  voice.openCaptures.set(kind, epoch);
  // Stopping this kind, leaving the seat and a newer start all move the
  // epoch on. A rebuild of the seat's connection does not: the picker or
  // prompt may answer whenever the user is done with it.
  const newest = () => videoEpoch(kind) === epoch;
  // A display capture owns its video and optional browser-selected audio.
  let stream: MediaStream;
  try {
    stream = await captureVideo(kind);
  } catch (error) {
    if (isOverconstrainedError(error) && newest()) {
      voice.deps?.onError?.(
        new Error(
          kind === "v"
            ? "Die Kamera unterstützt das Streamprofil nicht."
            : "Die Bildschirmaufnahme unterstützt das Streamprofil nicht.",
        ),
      );
    } else if (kind === "v" && cameraUnavailable(error) && newest()) {
      // Otherwise the button just springs back and nothing says why.
      voice.deps?.onError?.(new Error("Die Kamera ist nicht verfügbar."));
    } else if (kind !== "v" && !displayCancelled(error) && newest()) {
      // The same for a Live: only a cancel needs no word.
      logVoice("warn", "capture", {
        track: kind,
        detail:
          error instanceof Error
            ? `${error.name}: ${error.message}`
            : String(error),
      });
      voice.deps?.onError?.(displayFailure(error));
    }
    if (kind === "v" && newest()) {
      useVoice.setState({ camera: false });
    }
    if (kind === "s" && newest()) {
      useVoice.setState({ sharing: false });
    }
    return;
  } finally {
    if (voice.openCaptures.get(kind) === epoch) voice.openCaptures.delete(kind);
  }
  voice.pendingDisplayStreams.delete(stream);
  if (!newest()) {
    stopTracks(stream);
    return;
  }
  // The source can end while its profile is still being applied ("stop
  // sharing", a closed portal session). Its "ended" event is gone by now, so
  // nothing would ever take this capture down again.
  if (!hasLiveTrack(stream, "video")) {
    stopTracks(stream);
    useVoice.setState(kind === "v" ? { camera: false } : { sharing: false });
    return;
  }
  const self = currentUserId();
  if (kind === "s") {
    const sound = voice.displaySound.get(stream);
    if (!sharesSourceAudio()) {
      // Switched off while the picker or the capture was still busy.
      stream.getAudioTracks().forEach((track) => track.stop());
      setSourceAudio("off");
    } else if (hasLiveTrack(stream, "audio")) setSourceAudio("sharing");
    else if (sound?.asked)
      setSourceAudio(
        "unavailable",
        sound.failed ? { failed: sound.failed } : null,
      );
    else setSourceAudio("off");
    for (const track of stream.getAudioTracks()) bindSourceAudio(stream, track);
  }
  bindEnded(stream, kind);
  if (kind === "v") {
    stopTracks(voice.cameraStream);
    voice.cameraStream = stream;
    useVoice.setState({ camera: true, localCamera: stream });
  } else {
    stopTracks(voice.screenStream);
    voice.screenStream = stream;
    stream.getVideoTracks().forEach((track) => hintTrack(track, "detail"));
    useVoice.setState({ sharing: true, localScreen: stream });
  }
  noteStreamProfileApply(kind === "v" ? "camera" : "screen", "idle");
  if (self) setPub(self, kind, true);
  noteStream(kind, true);
  // Yield so the local tile paints before publishing.
  await Promise.resolve();
  if (!newest()) return;
  // Without a connection that publishes, the seat's next one takes it.
  await publishLocal(kind, stream);
}

export function stopLocalVideo(kind: LocalVideo): void {
  if (kind === "s") {
    for (const pending of voice.pendingDisplayStreams) stopTracks(pending);
    voice.pendingDisplayStreams.clear();
  }
  const state = useVoice.getState();
  if (kind === "v" && !state.camera && !state.localCamera) return;
  if (kind === "s" && !state.sharing && !state.localScreen) return;
  if (kind === "v") {
    voice.cameraEpoch += 1;
    for (const pending of voice.pendingCameraStreams) stopTracks(pending);
    voice.pendingCameraStreams.clear();
  } else voice.screenEpoch += 1;
  const stream = kind === "v" ? voice.cameraStream : voice.screenStream;
  if (kind === "v") {
    voice.cameraStream = null;
    useVoice.setState({ camera: false, localCamera: null });
  } else {
    voice.screenStream = null;
    useVoice.setState({ sharing: false, localScreen: null });
  }
  const self = currentUserId();
  if (self) setPub(self, kind, false);
  sendPub(kind, false);
  const connection = voice.seat.connection;
  const kinds: TrackKind[] = kind === "v" ? [kind] : ["sa", kind];
  for (const source of kinds) {
    if (connection)
      void connection.closeSource(source).catch((error) => {
        if (voice.seat.connection === connection) voice.deps?.onError?.(error);
      });
  }
  stopTracks(stream);
  if (kind === "s") {
    setSourceAudio("off");
    if (self) setPub(self, "sa", false);
    sendPub("sa", false);
  }
  if (voice.seat.connection)
    void voice.videoBudget.enqueue(voice.seat.connection);
  noteStreamProfileApply(kind === "v" ? "camera" : "screen", "idle");
  noteStream(kind, false);
}

export async function publishLocal(
  kind: LocalVideo,
  stream: MediaStream,
): Promise<void> {
  const connection = publishingConnection();
  if (!connection) return;
  const mine = voice.seat.generation,
    epoch = videoEpoch(kind),
    uuid = captureEpoch(stream);
  const current = () =>
    voice.seat.connection === connection &&
    voice.seat.generation === mine &&
    videoEpoch(kind) === epoch;
  const video = stream
    .getVideoTracks()
    .find((track) => track.readyState !== "ended");
  if (!video) return;
  try {
    if (!current()) return;
    const sender = await connection.publish({
      kind,
      track: video,
      streamId: stream.id,
      epoch: uuid,
    });
    if (!current()) {
      if (connection.sender(kind) === sender)
        await connection.closeSource(kind, sender.producerId).catch(() => {});
      return;
    }
    const self = currentUserId();
    if (self) setPub(self, kind, true);
    sendPub(kind, true);
    if (kind === "s") {
      const audio = stream
        .getAudioTracks()
        .find((track) => track.readyState !== "ended");
      if (audio) {
        const audioSender = await connection.publish({
          kind: "sa",
          track: audio,
          streamId: stream.id,
          epoch: uuid,
          parent: sender.producerId,
        });
        if (!current()) {
          if (connection.sender("sa") === audioSender)
            await connection
              .closeSource("sa", audioSender.producerId)
              .catch(() => {});
          return;
        }
        if (audio.readyState === "ended") {
          // Switched off, or gone, while the publish was under way: the room
          // has been told so. Close this producer only, never a newer sound.
          if (connection.sender("sa") === audioSender)
            await connection
              .closeSource("sa", audioSender.producerId)
              .catch(() => {});
        } else {
          if (self) setPub(self, "sa", true);
          sendPub("sa", true);
        }
      }
    }
    if (current()) {
      await voice.videoBudget.enqueue(connection);
      await applySendBitrate();
    }
  } catch (error) {
    if (current()) {
      reportStreamOnce(error instanceof Error ? error.message : "produce");
      stopLocalVideo(kind);
    }
  }
}
