// The voice module's state and the small steps every part of a call uses:
// one VoiceSession object (see resetVoiceForTests), the user, the gateway
// announcements, the microphone's mute state and the send bitrates.

import type { MicProcessor } from "./audioProcessing.ts";
import { prioritizeSender } from "./mediaPriority.ts";
import { useSession } from "../auth/store.ts";
import { randomUuid } from "../lib/uuid.ts";
import type { Gateway } from "../ws/client.ts";
import type { TrackKind } from "../ws/protocol.ts";
import type { IceServer, OpenMedia } from "./media.ts";
import { MediaPeer, MediaRetry } from "./mediaPeer.ts";
import { ReceivedAudio } from "./receivedAudio.ts";
import { VideoLimits } from "./videoLimits.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { nativeGetDisplayMedia, nativeGetUserMedia } from "./native/capture.ts";
import type {
  MediaConnection,
  MediaConnectionOptions,
  MediaSender,
  ReceivedSource,
} from "./mediasoupConnection.ts";
import {
  type StreamProfileId,
  audioBitrate,
  sourceAudioBitrate,
  useMediaSettings,
} from "./settings.ts";
import { pushToTalkOpen } from "./pushToTalk.ts";
import { useVoice, type VoiceMic } from "./voiceStore.ts";

export type RtpSender = MediaSender;

export type VoiceGateway = Pick<
  Gateway,
  "send" | "onSig" | "onErr" | "onReady"
>;

export type VoiceDeps = {
  gateway: VoiceGateway;
  userId: () => string | null;
  createMediaConnection: (options: MediaConnectionOptions) => MediaConnection;
  getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
  getDisplayMedia: (
    constraints: MediaStreamConstraints,
  ) => Promise<MediaStream>;
  fetchTicket?: (channelId: string) => Promise<{
    ticket: string;
    media_path: string;
    ice_servers: IceServer[];
  }>;
  openMedia?: OpenMedia;
  onError?: (error: unknown) => void;
};

type MicGainInsert = MicProcessor;

/**
 * Everything the voice modules keep between calls of their functions. A
 * reset replaces the whole object, so a field added here is reset without
 * being listed anywhere else.
 */
export type VoiceSession = {
  deps: VoiceDeps | null;
  /** The gateway, settings, volume and device listeners are attached. */
  bound: boolean;
  unsubscribeUserVolumes: (() => void) | null;
  unsubscribePushToTalk: (() => void) | null;
  /** Remember a deliberate mute while temporarily deafened, also across
   * room switches. */
  preDeafenMuted: boolean;
  soundOnJoin: boolean;
  /** One stream toast per join. Further failures stay in the console. */
  streamReported: boolean;
  awaitingJoin: { serverId: string; channelId: string } | null;
  /** A Live (screen) the gateway has not confirmed yet: going live needs
   * `go_live`, and a refusal has to end the capture, not leave it running
   * for nobody. */
  awaitingScreen: { serverId: string; channelId: string } | null;
  republishOnJoin: boolean;
  announced: Set<TrackKind>;

  /** Seat media peer. `joinVoice` / `leaveVoice` hold this object. */
  seat: MediaPeer;
  seatRecoverySlot: RecoverySlot;
  seatRetry: MediaRetry;
  /** Seat connections whose `start` is through. Before that a connection is
   * the seat's already, for the server's events, but its device has not
   * loaded and it turns every publication down. */
  startedConnections: WeakSet<MediaConnection>;
  videoBudget: VideoLimits;
  /** Each browser capture lifetime has one UUID, shared with its source
   * audio. */
  captureEpochs: WeakMap<MediaStream, string>;

  localStream: MediaStream | null;
  rawMicStream: MediaStream | null;
  activeMicGain: MicGainInsert | null;
  micForceBrowser: boolean;
  micEpoch: number;
  /** A microphone prompt that a seat rebuild overtook. Its answer belonged to
   * the old connection and was dropped with it, and a rebuild does not ask for
   * the microphone on its own: the next connection has to. */
  micOvertaken: boolean;
  /** Which join a microphone prompt belongs to. A rebuild keeps it, a new
   * join (another channel, or leaving and joining again) moves it on, so an
   * answer from an earlier join never takes over the current one's capture. */
  micJoin: number;
  /** Why the call has no microphone, until one is captured again. */
  micFailure: "unavailable" | "denied" | null;
  /** The chosen microphone went away and the default one stands in: it comes
   * back when the chosen one does. */
  micFellBack: boolean;
  /** Captures whose loss has been handled already: unplugging fires both the
   * track's `ended` and the device list's `devicechange`. */
  micLossHandled: WeakSet<MediaStream>;
  pendingMicRaw: Set<MediaStream>;
  requestedMicPause: WeakMap<
    MediaConnection,
    { producer: string; paused: boolean }
  >;
  audioCommitChain: Promise<void>;

  cameraStream: MediaStream | null;
  screenStream: MediaStream | null;
  cameraEpoch: number;
  screenEpoch: number;
  cameraProfileEpoch: number;
  screenProfileEpoch: number;
  cameraCommitChain: Promise<void>;
  pendingCameraStreams: Set<MediaStream>;
  pendingDisplayStreams: Set<MediaStream>;
  /** Epoch of the start whose picker or camera prompt has not answered yet. */
  openCaptures: Map<LocalVideo, number>;
  /** Whether a display capture was asked for sound, and the desktop app's
   * reason when it could not give it. */
  displaySound: WeakMap<MediaStream, { asked: boolean; failed?: string }>;
  /** Latest change of the sound of a running Live; older ones give way. */
  sourceAudioChange: number;

  receivedIdentity: WeakMap<MediaStreamTrack, ReceivedSource>;
  receivedAudio: ReceivedAudio;
  /** Video still arriving on the open media peer, keyed by publisher. */
  receivedVideo: Map<string, Partial<Record<"v" | "s", MediaStream>>>;
};

/** The epochs an answer is held to. They only ever move on, also across a
 * reset: an answer from before it must not match the new session's. */
export const VOICE_EPOCHS = [
  "micEpoch",
  "micJoin",
  "cameraEpoch",
  "screenEpoch",
  "cameraProfileEpoch",
  "screenProfileEpoch",
  "sourceAudioChange",
] as const;

export function createVoiceSession(previous?: VoiceSession): VoiceSession {
  const receivedIdentity = new WeakMap<MediaStreamTrack, ReceivedSource>();
  const session: VoiceSession = {
    deps: null,
    bound: false,
    unsubscribeUserVolumes: null,
    unsubscribePushToTalk: null,
    preDeafenMuted: false,
    soundOnJoin: false,
    streamReported: false,
    awaitingJoin: null,
    awaitingScreen: null,
    republishOnJoin: false,
    announced: new Set(),

    seat: new MediaPeer(),
    seatRecoverySlot: new Map(),
    seatRetry: new MediaRetry(),
    startedConnections: new WeakSet(),
    videoBudget: new VideoLimits(() => session.seat, profileForVideoTrack),
    captureEpochs: new WeakMap(),

    localStream: null,
    rawMicStream: null,
    activeMicGain: null,
    micForceBrowser: false,
    micEpoch: 0,
    micOvertaken: false,
    micJoin: 0,
    micFailure: null,
    micFellBack: false,
    micLossHandled: new WeakSet(),
    pendingMicRaw: new Set(),
    requestedMicPause: new WeakMap(),
    audioCommitChain: Promise.resolve(),

    cameraStream: null,
    screenStream: null,
    cameraEpoch: 0,
    screenEpoch: 0,
    cameraProfileEpoch: 0,
    screenProfileEpoch: 0,
    cameraCommitChain: Promise.resolve(),
    pendingCameraStreams: new Set(),
    pendingDisplayStreams: new Set(),
    openCaptures: new Map(),
    displaySound: new WeakMap(),
    sourceAudioChange: 0,

    receivedIdentity,
    receivedAudio: new ReceivedAudio({
      identity: (track) => receivedIdentity.get(track),
      self: currentUserId,
      voiceAttached: applyLocalAudio,
    }),
    receivedVideo: new Map(),
  };
  if (previous) for (const key of VOICE_EPOCHS) session[key] = previous[key];
  return session;
}

export let voice = createVoiceSession();

/** A fresh session in place of the current one, which it returns: after a
 * reset nothing of a call is left (resetVoiceForTests). */
export function renewVoiceSession(): VoiceSession {
  const previous = voice;
  voice = createVoiceSession(previous);
  return previous;
}

export function captureEpoch(stream: MediaStream): string {
  let epoch = voice.captureEpochs.get(stream);
  if (!epoch) {
    epoch = randomUuid();
    voice.captureEpochs.set(stream, epoch);
  }
  return epoch;
}

export type Direction = "send" | "recv";

export type RecoverySlot = Map<Direction, ReturnType<typeof setTimeout>>;

export function logVoice(
  level: "info" | "warn",
  step: string,
  extra?: Record<string, string | number | boolean | undefined>,
): void {
  const state = useVoice.getState();
  const fields = {
    step,
    user: currentUserId() ?? undefined,
    channel: state.channelId ?? undefined,
    server: state.serverId ?? undefined,
    ...extra,
  };
  if (level === "warn") console.warn("[gelabber:voice]", fields);
  else console.info("[gelabber:voice]", fields);
}

export function currentUserId(): string | null {
  return voice.deps?.userId && voice.deps.userId !== currentUserId
    ? voice.deps.userId()
    : (useSession.getState().user?.id ?? null);
}

export async function defaultGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  if (isDesktopApp()) return nativeGetUserMedia(constraints);
  return navigator.mediaDevices.getUserMedia(constraints);
}

export async function defaultGetDisplayMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  if (isDesktopApp()) return nativeGetDisplayMedia(constraints);
  return navigator.mediaDevices.getDisplayMedia(constraints);
}

export function hintTrack(
  track: MediaStreamTrack,
  hint: "speech" | "detail" | "music",
): void {
  try {
    (track as MediaStreamTrack & { contentHint?: string }).contentHint = hint;
  } catch {
    // contentHint is best-effort
  }
}

export function setPub(userId: string, kind: TrackKind, on: boolean): void {
  const state = useVoice.getState();
  const current = state.participants[userId] ?? { pubs: [] };
  const has = current.pubs.includes(kind);
  const pubs = on
    ? has
      ? current.pubs
      : [...current.pubs, kind]
    : current.pubs.filter((item) => kind !== item);
  useVoice.setState({
    participants: {
      ...state.participants,
      [userId]: { pubs },
    },
  });
}

export function stopTracks(stream: MediaStream | null): void {
  stream?.getTracks().forEach((track) => track.stop());
}

/** What the client captures itself: the camera, or a Live. */
export type LocalVideo = "v" | "s";

/** The seat's connection once it takes publications. What is captured or
 * changed while it starts is published by `startPeer` when the start is
 * through. */
export function publishingConnection(): MediaConnection | null {
  const connection = voice.seat.connection;
  return connection && voice.startedConnections.has(connection)
    ? connection
    : null;
}

export function hasLiveTrack(
  stream: MediaStream | null,
  kind: "audio" | "video",
): boolean {
  return Boolean(
    stream
      ?.getTracks()
      .some((track) => track.kind === kind && track.readyState !== "ended"),
  );
}

/** The microphone state the call bar shows. */
export function syncMicState(): void {
  const mic: VoiceMic =
    voice.micFailure ?? (pushToTalkOpen() ? "live" : "ptt-idle");
  if (useVoice.getState().mic !== mic) useVoice.setState({ mic });
}

export function applyLocalAudio(): void {
  const state = useVoice.getState();
  const micOff = state.muted || state.deafened;
  // Push-to-talk with its key up silences the track only: the producer
  // stays resumed, so pressing the key needs no round trip to the server.
  const silent = micOff || !pushToTalkOpen();
  syncMicState();
  voice.localStream?.getAudioTracks().forEach((track) => {
    track.enabled = !silent;
  });
  voice.rawMicStream?.getAudioTracks().forEach((track) => {
    track.enabled = !silent;
  });
  const connection = voice.seat.connection;
  const sender = connection?.sender("a");
  if (connection && sender) {
    const previous = voice.requestedMicPause.get(connection);
    if (
      previous?.producer !== sender.producerId ||
      previous.paused !== micOff
    ) {
      const request = { producer: sender.producerId, paused: micOff };
      voice.requestedMicPause.set(connection, request);
      void connection.setSourcePaused("a", micOff).catch((error) => {
        if (voice.requestedMicPause.get(connection) === request)
          voice.requestedMicPause.delete(connection);
        if (voice.seat.connection === connection) voice.deps?.onError?.(error);
      });
    }
  }
  voice.receivedAudio.applyPlayback();
}

export async function applySendBitrate(): Promise<void> {
  const pc = voice.seat.connection,
    generation = voice.seat.generation;
  const bitrate = audioBitrate();
  for (const sender of voice.seat.connection?.senders() ?? []) {
    if (sender.track?.kind !== "audio") continue;
    const params = sender.getParameters?.();
    if (!params?.encodings.length) continue;
    for (const encoding of params.encodings) {
      const cap = sender.sourceKind === "sa" ? sourceAudioBitrate() : bitrate;
      if (cap === null) delete encoding.maxBitrate;
      else encoding.maxBitrate = cap;
    }
    try {
      await sender.setParameters?.(params);
    } catch {
      // Chromium rejects setParameters before the first description.
    }
    await prioritizeSender(
      sender,
      "audio",
      () =>
        voice.seat.connection === pc && voice.seat.generation === generation,
    );
  }
}

export function profileForVideoTrack(track: MediaStreamTrack): StreamProfileId {
  const camera = voice.cameraStream
    ?.getVideoTracks()
    .some((item) => item === track);
  if (camera) return useMediaSettings.getState().cameraProfile;
  return useMediaSettings.getState().screenProfile;
}

export function sendPub(kind: TrackKind, on: boolean): void {
  const state = useVoice.getState();
  if (!state.serverId || !state.channelId) return;
  if (on) voice.announced.add(kind);
  else voice.announced.delete(kind);
  if (kind === "s")
    voice.awaitingScreen = on
      ? { serverId: state.serverId, channelId: state.channelId }
      : null;
  voice.deps?.gateway.send({
    op: "sig",
    t: on ? "p" : "u",
    s: state.serverId,
    c: state.channelId,
    k: kind,
  });
}

const STREAM_TOAST =
  "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv.";

export function reportStreamOnce(detail?: string): void {
  logVoice("warn", "stream", { detail });
  if (voice.streamReported) return;
  voice.streamReported = true;
  voice.deps?.onError?.(new Error(STREAM_TOAST));
}
