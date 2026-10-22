import {
  captureMicrophone,
  createProcessor,
  noteAudioProcessing,
  useAudioProcessing,
  type MicProcessor,
} from "./audioProcessing.ts";
import { prioritizeSender, readSenderPriority } from "./mediaPriority.ts";
import {
  playCallSound,
  unlockCallSounds,
  stopCallSounds,
  setCallSoundsDeafened,
} from "./callSounds.ts";
// Local voice state, capture and playback; mediasoup owns browser transport.
// Join updates the store immediately; ticket / ICE / getUserMedia run after.
// Camera and Live (a screen): local preview first, publish on the media path.
// An older client's Go Live is received like a Live. Chat WS: presence
// (j/l/p/u).
// Own versioned Media WS: public mediasoup-client transport and source signaling.

import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { useSession } from "../auth/store.ts";
import { notifyError } from "../components/toasts.ts";
import { randomUuid } from "../lib/uuid.ts";
import { getGateway, type Gateway } from "../ws/client.ts";
import type { ErrFrame, SigEvent, TrackKind } from "../ws/protocol.ts";
import { applyVoiceJoin, applyVoiceLeave } from "./roster.ts";
import {
  type IceServer,
  type MediaServerFrame,
  type OpenMedia,
  mediaWsUrl,
  openMediaSocket,
  requestMediaTicket,
  MEDIA_VERSION,
  MediaError,
} from "./media.ts";
import { MediaPeer, MediaRetry } from "./mediaPeer.ts";
import { ReceivedAudio } from "./receivedAudio.ts";
import { VideoLimits } from "./videoLimits.ts";
import { isDesktopApp } from "./native/bridge.ts";
import {
  captureAppAudio,
  nativeGetDisplayMedia,
  nativeGetUserMedia,
} from "./native/capture.ts";
import { loadNativeFeatures } from "./native/features.ts";
import { createStream } from "./native/tracks.ts";
import { resetPlaybackBoostForTests } from "./playbackBoost.ts";
import { useUserVolumes } from "./userVolume.ts";
import {
  createMediaConnection,
  preloadMediaSdk,
  type MediaConnection,
  type MediaConnectionOptions,
  type MediaSender,
  type ReceivedSource,
} from "./mediasoupConnection.ts";
import type { ProducerCodecOptions } from "mediasoup-client/types";
import {
  attachDiagnostics,
  defaultCaps,
  detachDiagnostics,
  installDiagnosticsLogoutReset,
  noteDiagnosticEvent,
  resetDiagnostics,
  statsEntriesFromReport,
  type Caps,
  type VideoSource,
  type AudioSource,
} from "./diagnostics.ts";
import {
  type MediaSettings,
  type StreamKind,
  type StreamProfileId,
  audioBitrate,
  sourceAudioBitrate,
  SOURCE_AUDIO_CONSTRAINTS,
  isOverconstrainedError,
  noteStreamProfileApply,
  onMediaSettingsChange,
  sharesSourceAudio,
  streamProfileFps,
  useMediaSettings,
  videoConstraintLadder,
  videoConstraintsFor,
} from "./settings.ts";
import {
  configurePushToTalk,
  onPushToTalkGate,
  pushToTalkOpen,
} from "./pushToTalk.ts";
import {
  IDLE_VOICE,
  useVoice,
  type SourceAudioNote,
  type SourceAudioStatus,
  type VoiceConnection,
  type VoiceMic,
  type VoiceParticipant,
  type VoiceState,
} from "./voiceStore.ts";

export {
  useVoice,
  type LostCall,
  type RemoteVideo,
  type SourceAudioNote,
  type SourceAudioStatus,
  type VoiceConnection,
  type VoiceMic,
  type VoiceParticipant,
  type VoiceState,
  type VoiceStatus,
} from "./voiceStore.ts";

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
 * Everything this module keeps between calls of its functions. A reset
 * replaces the whole object, so a field added here is reset without being
 * listed anywhere else.
 */
type VoiceSession = {
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
  receivedVideo: Map<string, Partial<Record<"v" | "s" | "l", MediaStream>>>;
};

/** The epochs an answer is held to. They only ever move on, also across a
 * reset: an answer from before it must not match the new session's. */
const VOICE_EPOCHS = [
  "micEpoch",
  "micJoin",
  "cameraEpoch",
  "screenEpoch",
  "cameraProfileEpoch",
  "screenProfileEpoch",
  "sourceAudioChange",
] as const;

function createVoiceSession(previous?: VoiceSession): VoiceSession {
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

let voice = createVoiceSession();

const LIVE_FORBIDDEN_MESSAGE = "In diesem Server darfst du nicht live gehen.";

function captureEpoch(stream: MediaStream): string {
  let epoch = voice.captureEpochs.get(stream);
  if (!epoch) {
    epoch = randomUuid();
    voice.captureEpochs.set(stream, epoch);
  }
  return epoch;
}

const ICE_DISCONNECTED_RESTART_DELAY_MS = 2000;
const ICE_RECOVERY_DEADLINE_MS = 10_000;
type Direction = "send" | "recv";
type RecoverySlot = Map<Direction, ReturnType<typeof setTimeout>>;
function clearRecovery(slot: RecoverySlot): void {
  for (const timer of slot.values()) clearTimeout(timer);
  slot.clear();
}
function clearSeatReconnectTimer(): void {
  clearRecovery(voice.seatRecoverySlot);
}

function logVoice(
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

function currentUserId(): string | null {
  return voice.deps?.userId && voice.deps.userId !== currentUserId
    ? voice.deps.userId()
    : (useSession.getState().user?.id ?? null);
}

async function defaultGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  if (isDesktopApp()) return nativeGetUserMedia(constraints);
  return navigator.mediaDevices.getUserMedia(constraints);
}

async function defaultGetDisplayMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  if (isDesktopApp()) return nativeGetDisplayMedia(constraints);
  return navigator.mediaDevices.getDisplayMedia(constraints);
}

/** Call synchronously from a click so the browser grants playback activation. */
export function retryPlayback(): void {
  voice.receivedAudio.retry();
}

/**
 * A phone call or the lock screen can pause call audio while the page is
 * hidden. Replay it once the page is back; where that needs a gesture, the
 * refused replay raises the "Ton starten" recovery instead of staying silent.
 */
function resumePlayback(): void {
  if (document.visibilityState === "visible") retryPlayback();
}

function hintTrack(
  track: MediaStreamTrack,
  hint: "speech" | "detail" | "music",
): void {
  try {
    (track as MediaStreamTrack & { contentHint?: string }).contentHint = hint;
  } catch {
    // contentHint is best-effort
  }
}

export function configureVoice(next: Partial<VoiceDeps>): void {
  const gateway = next.gateway ?? voice.deps?.gateway ?? getGateway();
  voice.deps = {
    gateway,
    userId: next.userId ?? voice.deps?.userId ?? currentUserId,
    createMediaConnection:
      next.createMediaConnection ??
      voice.deps?.createMediaConnection ??
      createMediaConnection,
    getUserMedia:
      next.getUserMedia ?? voice.deps?.getUserMedia ?? defaultGetUserMedia,
    getDisplayMedia:
      next.getDisplayMedia ??
      voice.deps?.getDisplayMedia ??
      defaultGetDisplayMedia,
    fetchTicket:
      next.fetchTicket ?? voice.deps?.fetchTicket ?? requestMediaTicket,
    openMedia: next.openMedia ?? voice.deps?.openMedia ?? openMediaSocket,
    onError: next.onError ?? voice.deps?.onError ?? notifyError,
  };
  ensureBound();
}

function ensureBound(): void {
  if (voice.bound) return;
  const gateway = voice.deps?.gateway ?? getGateway();
  if (!voice.deps) {
    voice.deps = {
      gateway,
      userId: currentUserId,
      createMediaConnection,
      getUserMedia: defaultGetUserMedia,
      getDisplayMedia: defaultGetDisplayMedia,
      fetchTicket: requestMediaTicket,
      openMedia: openMediaSocket,
      onError: notifyError,
    };
  }
  gateway.onSig(onSig);
  gateway.onErr(onErr);
  gateway.onReady(onReady);
  onMediaSettingsChange(handleSettingsChange);
  voice.unsubscribeUserVolumes?.();
  voice.unsubscribeUserVolumes = useUserVolumes.subscribe(() =>
    voice.receivedAudio.applyUserVolumes(),
  );
  onPushToTalkGate(applyLocalAudio);
  voice.unsubscribePushToTalk?.();
  voice.unsubscribePushToTalk = useVoice.subscribe((next, prev) => {
    if (next.status !== prev.status) {
      syncPushToTalk();
      bindMediaSession(next.status === "joined");
    }
    if (
      next.status !== prev.status ||
      next.muted !== prev.muted ||
      next.deafened !== prev.deafened
    )
      syncMicrophoneActive(next);
  });
  syncPushToTalk();
  // Re-adding the same listener after a rebind is a no-op.
  if (typeof document !== "undefined")
    document.addEventListener("visibilitychange", resumePlayback);
  if (typeof navigator !== "undefined")
    navigator.mediaDevices?.addEventListener?.("devicechange", onDeviceChange);
  voice.bound = true;
}

type CallMediaSession = MediaSession & {
  setMicrophoneActive?: (active: boolean) => Promise<void> | void;
};

function callMediaSession(): CallMediaSession | null {
  return typeof navigator !== "undefined" && navigator.mediaSession
    ? navigator.mediaSession
    : null;
}

/** Headset buttons and the system's call controls: mute and hang up while
 * in a call. */
function bindMediaSession(joined: boolean): void {
  const session = callMediaSession();
  if (!session) return;
  const handlers = [
    ["togglemicrophone", () => toggleMute()],
    ["hangup", () => leaveVoice()],
  ] as const;
  for (const [action, handler] of handlers) {
    try {
      // Not in the DOM typings yet; Chromium has them for calls.
      session.setActionHandler(
        action as MediaSessionAction,
        joined ? handler : null,
      );
    } catch {
      // Browsers without call actions throw for them.
    }
  }
}

function syncMicrophoneActive(state: VoiceState): void {
  const session = callMediaSession();
  if (!session?.setMicrophoneActive || state.status !== "joined") return;
  try {
    void Promise.resolve(
      session.setMicrophoneActive(!(state.muted || state.deafened)),
    ).catch(() => undefined);
  } catch {
    // Best effort, like the actions.
  }
}

function upsert(
  participants: Record<string, VoiceParticipant>,
  userId: string,
): Record<string, VoiceParticipant> {
  if (participants[userId]) return participants;
  return { ...participants, [userId]: { pubs: [] } };
}

function setPub(userId: string, kind: TrackKind, on: boolean): void {
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

function noteReceived(
  userId: string,
  kind: "v" | "s" | "l",
  stream: MediaStream,
  track: MediaStreamTrack,
): void {
  // Retire an old tile alias before binding the SDK Consumer's current source.
  for (const [publisher, sources] of voice.receivedVideo) {
    for (const previousKind of ["v", "s", "l"] as const) {
      if (
        (publisher !== userId || previousKind !== kind) &&
        sources[previousKind]?.getVideoTracks().includes(track)
      ) {
        forgetReceived(publisher, previousKind);
      }
    }
  }
  const prev = voice.receivedVideo.get(userId) ?? {};
  voice.receivedVideo.set(userId, { ...prev, [kind]: stream });
  track.addEventListener("ended", () => {
    const held = voice.receivedVideo.get(userId);
    if (!held || held[kind] !== stream) return;
    const next = { ...held };
    delete next[kind];
    if (!next.v && !next.s && !next.l) voice.receivedVideo.delete(userId);
    else voice.receivedVideo.set(userId, next);
    dropRemote(userId, kind);
  });
}

/** An explicit unpublish ends that tile. A later `t:"p"` must not restore it. */
function forgetReceived(userId: string, kind: "v" | "s" | "l"): void {
  const held = voice.receivedVideo.get(userId);
  if (held?.[kind]) {
    const next = { ...held };
    delete next[kind];
    if (!next.v && !next.s && !next.l) voice.receivedVideo.delete(userId);
    else voice.receivedVideo.set(userId, next);
  }
  dropRemote(userId, kind);
}

function reattachReceived(userId: string, kind: "v" | "s" | "l"): void {
  const stream = voice.receivedVideo.get(userId)?.[kind];
  if (!stream) return;
  const dead = stream
    .getVideoTracks()
    .some((track) => track.readyState === "ended");
  if (dead) return;
  const state = useVoice.getState();
  if (
    kind !== "v" &&
    state.sourceWatchSupported &&
    !state.sourceSubscriptions[userId]?.[kind]
  )
    return;
  const current = state.remote[userId] ?? {};
  if (current[kind] === stream) return;
  useVoice.setState({
    remote: {
      ...state.remote,
      [userId]: { ...current, [kind]: stream },
    },
  });
}

function clearReceived(): void {
  voice.receivedVideo.clear();
}

function dropRemote(userId: string, kind?: "v" | "s" | "l"): void {
  const state = useVoice.getState();
  if (!state.remote[userId]) return;
  if (!kind) {
    const next = { ...state.remote };
    delete next[userId];
    useVoice.setState({ remote: next });
    return;
  }
  const current = { ...state.remote[userId] };
  if (kind) delete current[kind];
  const next = { ...state.remote };
  if (!current.v && !current.s && !current.l) {
    delete next[userId];
  } else {
    next[userId] = current;
  }
  useVoice.setState({ remote: next });
}

function onSig(event: SigEvent): void {
  const state = useVoice.getState();
  const inRoom =
    state.status === "joined" &&
    event.s === state.serverId &&
    event.c === state.channelId;
  if (!inRoom) {
    return;
  }
  const userId = event.u;
  if (!userId) return;
  switch (event.t) {
    case "j":
      if (
        userId !== currentUserId() &&
        !event.replay &&
        !state.participants[userId]
      ) {
        playCallSound("join");
      }
      if (
        userId === currentUserId() &&
        voice.awaitingJoin &&
        event.c === voice.awaitingJoin.channelId
      ) {
        voice.awaitingJoin = null;
        if (voice.soundOnJoin) {
          voice.soundOnJoin = false;
          playCallSound("join");
        }
        if (voice.republishOnJoin) {
          voice.republishOnJoin = false;
          reannounceActive();
        }
      }
      useVoice.setState({
        participants: upsert(state.participants, userId),
      });
      return;
    case "l":
      if (userId === currentUserId()) {
        return;
      }
      {
        if (state.participants[userId]) playCallSound("leave");
        const next = { ...state.participants };
        delete next[userId];
        useVoice.setState({ participants: next });
        // Gateway absence is not the end of the media track. The tile
        // stays until the receiver track or this peer connection ends.
      }
      return;
    case "p":
    case "u": {
      if (!event.k) return;
      if (event.t === "p" && event.k === "s" && userId === currentUserId())
        voice.awaitingScreen = null;
      const current = state.participants[userId] ?? { pubs: [] };
      const pubs =
        event.t === "p"
          ? current.pubs.includes(event.k)
            ? current.pubs
            : [...current.pubs, event.k]
          : current.pubs.filter((kind) => kind !== event.k);
      useVoice.setState({
        participants: {
          ...state.participants,
          [userId]: { pubs },
        },
      });
      if (event.t === "u" && (event.k === "sa" || event.k === "la"))
        voice.receivedAudio.dropSource(userId, event.k === "sa" ? "s" : "l");
      if (event.k === "v" || event.k === "s" || event.k === "l") {
        if (event.t === "u") {
          // Gateway `t:"l"` keeps a track that is still arriving. `t:"u"`
          // means that the camera or a Live actually ended.
          forgetReceived(userId, event.k);
          if (event.k !== "v") voice.receivedAudio.dropSource(userId, event.k);
        } else if (userId !== currentUserId()) {
          reattachReceived(userId, event.k);
        }
      }
      return;
    }
    default:
      return;
  }
}

function onErr(err: ErrFrame): void {
  const pending = voice.awaitingJoin;
  if (
    pending &&
    err.s === pending.serverId &&
    err.c === pending.channelId &&
    (err.e === "forbidden" || err.e === "not_found" || err.e === "bad_request")
  ) {
    const state = useVoice.getState();
    if (state.channelId === pending.channelId) {
      voice.awaitingJoin = null;
      voice.awaitingScreen = null;
      stopPeer();
      useVoice.setState({
        ...IDLE_VOICE,
        playbackBlocked: state.playbackBlocked,
      });
      const code: ApiErrorCode =
        err.e === "forbidden" ||
        err.e === "not_found" ||
        err.e === "bad_request"
          ? err.e
          : "bad_request";
      voice.deps?.onError?.(new ApiError(code, 0, errorMessage(code)));
    }
    return;
  }
  if (
    voice.awaitingScreen &&
    !voice.awaitingJoin &&
    err.s === voice.awaitingScreen.serverId &&
    err.c === voice.awaitingScreen.channelId &&
    err.e === "forbidden"
  ) {
    // Refused to go live: the seat stays, the screen capture ends.
    voice.awaitingScreen = null;
    if (useVoice.getState().sharing) stopLocalVideo("s");
    voice.deps?.onError?.(new Error(LIVE_FORBIDDEN_MESSAGE));
  }
}

function onReady(): void {
  voice.soundOnJoin = false;
  stopCallSounds();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  const self = currentUserId();
  const keepMedia = voice.seat.isOpen();
  useVoice.setState({
    participants: self
      ? { [self]: state.participants[self] ?? { pubs: [] } }
      : {},
    // A live Consumer does not emit another source callback. Keep the
    // streams VoiceRoom is already showing.
    remote: keepMedia ? state.remote : {},
  });
  occupySelf(state);
  voice.awaitingJoin = { serverId: state.serverId, channelId: state.channelId };
  voice.republishOnJoin = keepMedia;
  voice.deps?.gateway.send({
    op: "sig",
    t: "j",
    s: state.serverId,
    c: state.channelId,
  });
  if (state.muted) sendFlag("m", state.serverId, state.channelId, true);
  if (state.deafened) sendFlag("d", state.serverId, state.channelId, true);
  // Rebuild only when the media transport has actually closed or failed.
  if (keepMedia) return;
  scheduleSeatRebuild();
}

function stopTracks(stream: MediaStream | null): void {
  stream?.getTracks().forEach((track) => track.stop());
}

/** What the client captures itself: the camera, or a Live. */
type LocalVideo = "v" | "s";

function noteStream(kind: LocalVideo, started: boolean): void {
  const state = useVoice.getState();
  noteDiagnosticEvent({
    kind: started ? "stream-start" : "stream-stop",
    connection: "voice",
    detail: kind === "v" ? "camera" : "screen",
    streaming: state.camera || state.sharing,
  });
}

function diagnosticVideoSources(): Record<string, VideoSource> {
  const sources: Record<string, VideoSource> = {};
  const camera = voice.cameraStream?.getVideoTracks()[0];
  const screen = voice.screenStream?.getVideoTracks()[0];
  if (camera) sources[camera.id] = "camera";
  if (screen) sources[screen.id] = "screen";
  for (const source of voice.seat.connection?.consumers() ?? []) {
    if (source.k === "v") sources[source.track.id] = "camera";
    if (source.k === "s") sources[source.track.id] = "screen";
    if (source.k === "l") sources[source.track.id] = "live";
  }
  return sources;
}

function diagnosticAudioSources(): Record<string, AudioSource> {
  const sources: Record<string, AudioSource> = {};
  for (const track of voice.screenStream?.getAudioTracks() ?? [])
    sources[track.id] = "screen-audio";
  voice.receivedAudio.diagnosticSources(sources);
  return sources;
}

/** The seat's microphones for the speaking indicator: the own sender and
 * every received microphone, never screen or Live sound. */
export function voiceMicrophones(): {
  self: string | null;
  sender: MediaSender | undefined;
  received: ReceivedSource[];
} {
  const connection = voice.seat.connection;
  return {
    self: currentUserId(),
    sender: connection?.sender("a"),
    received: (connection?.consumers() ?? []).filter(
      (source) => source.kind === "audio" && source.k === "a",
    ),
  };
}

function voiceStreaming(): boolean {
  const state = useVoice.getState();
  return state.camera || state.sharing;
}

function voiceCaps(): Caps {
  const base = defaultCaps();
  const senders = (voice.seat.connection?.senders() ?? []).filter(
    (sender) =>
      sender.track?.kind === "video" && sender.track.readyState !== "ended",
  );
  const profiles = senders.map((sender) => profileForVideoTrack(sender.track!));
  const videoLimits: NonNullable<Caps["videoLimits"]> = {};
  for (const [index, sender] of senders.entries()) {
    const source = sender.track
      ? diagnosticVideoSources()[sender.track.id]
      : undefined;
    if (!source) continue;
    const encodings = sender.getParameters?.().encodings;
    const hasBitrates =
      encodings?.length &&
      encodings.every(
        (encoding) =>
          typeof encoding.maxBitrate === "number" &&
          Number.isFinite(encoding.maxBitrate) &&
          encoding.maxBitrate > 0,
      );
    const maxBitrate = hasBitrates
      ? encodings.reduce((sum, encoding) => sum + encoding.maxBitrate!, 0)
      : null;
    const maxFps =
      Math.max(
        0,
        ...(encodings ?? []).map((encoding) => encoding.maxFramerate ?? 0),
      ) || streamProfileFps(profiles[index] ?? "balanced");
    videoLimits[source] = { maxBitrate, maxFps };
  }
  const audioLimits: NonNullable<Caps["audioLimits"]> = {};
  for (const sender of voice.seat.connection?.senders() ?? []) {
    if (sender.track?.kind !== "audio") continue;
    const encodings = sender.getParameters?.().encodings;
    if (!encodings?.length) continue;
    const kind = sender.sourceKind;
    const source: AudioSource =
      kind === "sa" ? "screen-audio" : kind === "la" ? "live-audio" : "voice";
    audioLimits[source] = encodings.every(
      (encoding) => typeof encoding.maxBitrate === "number",
    )
      ? encodings.reduce((sum, encoding) => sum + encoding.maxBitrate!, 0)
      : null;
  }
  const active = Object.values(videoLimits);
  const senderPriorities: NonNullable<Caps["senderPriorities"]> = {};
  for (const sender of voice.seat.connection?.senders() ?? []) {
    if (!sender.track) continue;
    const kind = sender.sourceKind;
    const source =
      sender.track.kind === "audio"
        ? kind === "sa"
          ? "screen-audio"
          : kind === "la"
            ? "live-audio"
            : "voice"
        : diagnosticVideoSources()[sender.track.id];
    if (source) senderPriorities[source] = readSenderPriority(sender);
  }
  return {
    ...base,
    audioLimits,
    senderPriorities,
    sourceAudioMaxBitrate: sourceAudioBitrate(),
    videoSendBudget: active.length
      ? active.every((cap) => cap.maxBitrate !== null)
        ? active.reduce((sum, cap) => sum + (cap.maxBitrate ?? 0), 0)
        : null
      : base.videoSendBudget,
    videoMaxFps: active.length
      ? Math.max(...active.map((cap) => cap.maxFps))
      : base.videoMaxFps,
    videoLimits,
  };
}

function attachSeatDiagnostics(generation: number): void {
  attachDiagnostics({
    role: "voice",
    caps: voiceCaps,
    streaming: voiceStreaming,
    videoSources: diagnosticVideoSources,
    audioSources: diagnosticAudioSources,
    getReport: async () => {
      const pc = voice.seat.connection;
      if (voice.seat.generation !== generation || !pc?.getStats) return null;
      try {
        return statsEntriesFromReport(await pc.getStats());
      } catch {
        return null;
      }
    },
  });
}

function codecOptionsFor(kind: TrackKind): ProducerCodecOptions {
  if (kind === "v" || kind === "s" || kind === "l") return {};
  const source = kind === "sa" || kind === "la";
  const original = useMediaSettings.getState().processingMode === "original";
  const bitrate = source ? sourceAudioBitrate() : audioBitrate();
  return {
    opusFec: true,
    opusDtx: !source && !original,
    opusStereo: source || original,
    // A missing Opus hint selects libwebrtc's low default even without an RTP cap.
    opusMaxAverageBitrate: bitrate ?? (source || original ? 510_000 : 128_000),
  };
}
function receiveSource(source: ReceivedSource): void {
  voice.receivedIdentity.set(source.track, source);
  attachIncoming(source.track, source.stream);
}
function closeReceivedSource(source: ReceivedSource): void {
  if (voice.receivedIdentity.get(source.track) !== source) return;
  voice.receivedIdentity.delete(source.track);
  voice.receivedAudio.close(source.track, source.kind === "audio");
  if (source.kind === "audio") return;
  if (source.k !== "v" && source.k !== "s" && source.k !== "l") return;
  const held = voice.receivedVideo.get(source.owner)?.[source.k];
  if (held === source.stream) {
    forgetReceived(source.owner, source.k);
    dropRemote(source.owner, source.k);
  }
}
function makeMediaConnection(
  peer: MediaPeer,
  generation: string,
  iceServers: IceServer[],
  mine: number,
): MediaConnection {
  const current = () => peer.generation === mine;
  const connection = (
    voice.deps?.createMediaConnection ?? createMediaConnection
  )({
    role: "voice",
    generation,
    iceServers,
    request: (method, data, deadlineEpochMs) =>
      peer.request(method, data, deadlineEpochMs),
    codecOptions: codecOptionsFor,
    onConsumer: (source) => {
      if (current()) receiveSource(source);
    },
    onConsumerClosed: (source) => {
      if (current()) closeReceivedSource(source);
    },
    onTransportState: (direction, state) => {
      if (current()) onTransportState(direction, state, mine);
    },
    onError: (error) => {
      if (current()) voice.deps?.onError?.(error);
    },
    onRepublished: () => {
      // New senders: the upload budget and priorities go on them again.
      if (!current() || !peer.connection) return;
      void voice.videoBudget.enqueue(peer.connection).catch(() => {});
      void applySendBitrate().catch(() => {});
    },
  });
  peer.connection = connection;
  return connection;
}

/** The seat's connection once it takes publications. What is captured or
 * changed while it starts is published by `startPeer` when the start is
 * through. */
function publishingConnection(): MediaConnection | null {
  const connection = voice.seat.connection;
  return connection && voice.startedConnections.has(connection)
    ? connection
    : null;
}

function hasLiveTrack(
  stream: MediaStream | null,
  kind: "audio" | "video",
): boolean {
  return Boolean(
    stream
      ?.getTracks()
      .some((track) => track.kind === kind && track.readyState !== "ended"),
  );
}

/** Reports the connection; observed only, the retries do not read it. */
function setConnection(connection: VoiceConnection): void {
  if (useVoice.getState().connection !== connection)
    useVoice.setState({ connection });
}

/** The microphone state the call bar shows. */
function syncMicState(): void {
  const mic: VoiceMic =
    voice.micFailure ?? (pushToTalkOpen() ? "live" : "ptt-idle");
  if (useVoice.getState().mic !== mic) useVoice.setState({ mic });
}

function stopPeer(preserveCapture = false): void {
  if (!preserveCapture) voice.micForceBrowser = false;
  if (!preserveCapture) {
    voice.micFailure = null;
    voice.micFellBack = false;
  }
  if (!preserveCapture) {
    voice.micOvertaken = false;
    voice.micJoin += 1;
  }
  if (!preserveCapture) voice.awaitingScreen = null;
  voice.seatRetry.cancel(!preserveCapture);
  // A rebuild keeps what is captured, and with it a picker or camera prompt
  // that is still open: what the user answers there goes to the next
  // connection.
  const open = new Set<LocalVideo>();
  if (preserveCapture) {
    for (const [kind, stream] of [
      ["v", voice.cameraStream],
      ["s", voice.screenStream],
    ] as const) {
      if (stream && !hasLiveTrack(stream, "video")) stopLocalVideo(kind);
    }
    for (const kind of ["v", "s"] as const)
      if (captureOpen(kind)) open.add(kind);
  }
  voice.streamReported = false;
  clearSeatReconnectTimer();
  clearRecovery(voice.seatRecoverySlot);
  detachDiagnostics("voice");
  voice.seat.close();
  voice.micEpoch += 1;
  voice.cameraProfileEpoch += 1;
  voice.screenProfileEpoch += 1;
  // The epoch is what an open capture is held to when it answers.
  for (const kind of ["v", "s"] as const)
    if (!open.has(kind)) bumpVideoEpoch(kind);
  voice.audioCommitChain = Promise.resolve();
  voice.cameraCommitChain = Promise.resolve();
  voice.videoBudget.reset();
  // The next connection announces what it publishes itself.
  voice.announced.clear();
  for (const stream of voice.pendingMicRaw) stopTracks(stream);
  voice.pendingMicRaw.clear();
  for (const stream of voice.pendingCameraStreams) stopTracks(stream);
  voice.pendingCameraStreams.clear();
  // Picked already; its sound or profile is still under way.
  if (!open.has("s")) {
    for (const stream of voice.pendingDisplayStreams) stopTracks(stream);
    voice.pendingDisplayStreams.clear();
  }
  if (
    !preserveCapture ||
    !hasLiveTrack(voice.localStream, "audio") ||
    (voice.rawMicStream && !hasLiveTrack(voice.rawMicStream, "audio"))
  ) {
    stopTracks(voice.localStream);
    stopTracks(voice.rawMicStream);
    disposeMicGain();
    voice.localStream = null;
    voice.rawMicStream = null;
  }
  if (!preserveCapture || !hasLiveTrack(voice.cameraStream, "video")) {
    stopTracks(voice.cameraStream);
    voice.cameraStream = null;
  }
  if (!preserveCapture || !hasLiveTrack(voice.screenStream, "video")) {
    stopTracks(voice.screenStream);
    voice.screenStream = null;
  }
  voice.receivedAudio.clear();
  clearReceived();
  useVoice.setState({
    localCamera: voice.cameraStream,
    localScreen: voice.screenStream,
    sourceAudio: preserveCapture
      ? useVoice.getState().sourceAudio
      : { s: "off" },
    sourceAudioNote: preserveCapture
      ? useVoice.getState().sourceAudioNote
      : { s: null },
    ...(preserveCapture
      ? {
          camera: !!voice.cameraStream || open.has("v"),
          sharing: !!voice.screenStream || open.has("s"),
        }
      : {}),
    remote: {},
  });
}

/** Leaves the seat after a failure. `lost` keeps the call to offer joining
 * it again, for a connection that failed rather than a refusal. */
function rollbackSeat(error?: unknown, lost = false): void {
  const state = useVoice.getState();
  const serverId = state.serverId;
  const channelId = state.channelId;
  voice.awaitingJoin = null;
  voice.republishOnJoin = false;
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...IDLE_VOICE,
    playbackBlocked: voice.receivedAudio.playbackBlocked,
    ...(lost && serverId && channelId
      ? {
          connection: "failed" as const,
          lostCall: {
            serverId,
            channelId,
            channelName: state.channelName ?? "Sprachkanal",
          },
        }
      : {}),
  });
  voice.receivedAudio.applyPlayback();
  if (serverId && channelId) {
    voice.deps?.gateway.send({
      op: "sig",
      t: "l",
      s: serverId,
      c: channelId,
    });
  }
  if (error !== undefined) {
    voice.deps?.onError?.(error);
  }
}

/** Push-to-talk follows the call and the settings: on in a call with the
 * mode chosen, off otherwise. */
function syncPushToTalk(): void {
  const settings = useMediaSettings.getState();
  configurePushToTalk(
    useVoice.getState().status === "joined" && settings.voiceInput === "ptt"
      ? { key: settings.pttKey, releaseDelay: settings.pttReleaseDelay }
      : null,
  );
}

function applyLocalAudio(): void {
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

async function applySendBitrate(): Promise<void> {
  const pc = voice.seat.connection,
    generation = voice.seat.generation;
  const bitrate = audioBitrate();
  for (const sender of voice.seat.connection?.senders() ?? []) {
    if (sender.track?.kind !== "audio") continue;
    const params = sender.getParameters?.();
    if (!params?.encodings.length) continue;
    for (const encoding of params.encodings) {
      const cap =
        sender.sourceKind === "sa" || sender.sourceKind === "la"
          ? sourceAudioBitrate()
          : bitrate;
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

function profileForVideoTrack(track: MediaStreamTrack): StreamProfileId {
  const camera = voice.cameraStream
    ?.getVideoTracks()
    .some((item) => item === track);
  if (camera) return useMediaSettings.getState().cameraProfile;
  return useMediaSettings.getState().screenProfile;
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
async function applyStreamProfile(kind: StreamKind): Promise<void> {
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

function disposeMicGain(): void {
  const insert = voice.activeMicGain;
  voice.activeMicGain = null;
  insert?.dispose();
  noteAudioProcessing(null);
}

function enqueueAudioCommit(job: () => Promise<void>): Promise<void> {
  const run = voice.audioCommitChain.then(job, job);
  voice.audioCommitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function enqueueCameraCommit(job: () => Promise<void>): Promise<void> {
  const run = voice.cameraCommitChain.then(job, job);
  voice.cameraCommitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function disableAudio(stream: MediaStream): void {
  stream.getAudioTracks().forEach((track) => {
    track.enabled = false;
  });
}

function micCurrent(
  pc: MediaConnection,
  session: number,
  request: number,
): boolean {
  return (
    voice.seat.connection === pc &&
    voice.seat.generation === session &&
    voice.micEpoch === request &&
    useVoice.getState().status === "joined"
  );
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

async function replaceSenderTrack(
  sender: RtpSender | undefined,
  track: MediaStreamTrack | null,
): Promise<void> {
  if (!sender?.replaceTrack) return;
  await sender.replaceTrack(track);
}

async function commitMicSend(
  pc: MediaConnection,
  send: MediaStream,
  session: number,
  capture: MediaStream = voice.rawMicStream ?? send,
): Promise<"replaced" | "added" | "none"> {
  send
    .getAudioTracks()
    .forEach((track) =>
      hintTrack(
        track,
        useMediaSettings.getState().processingMode === "original"
          ? "music"
          : "speech",
      ),
    );
  const track = send.getAudioTracks()[0];
  if (!track) return "none";
  if (voice.seat.connection !== pc || voice.seat.generation !== session)
    return "none";
  const existed = !!pc.sender("a");
  const state = useVoice.getState();
  await pc.publish({
    kind: "a",
    track,
    streamId: send.id,
    epoch: captureEpoch(capture),
    paused: state.muted || state.deafened,
  });
  return existed ? "replaced" : "added";
}

function failedMicProcessor(owner: MicProcessor): void {
  if (voice.activeMicGain !== owner) return;
  const sender = audioSender(),
    generation = voice.seat.generation,
    failedTrack = owner.stream.getAudioTracks()[0];
  // A capture that ended under the processor is the device gone (unplugged,
  // taken away), not the processor: the next capture keeps its filters.
  const lost =
    voice.rawMicStream !== null && !hasLiveTrack(voice.rawMicStream, "audio")
      ? voice.rawMicStream
      : null;
  if (!lost) voice.micForceBrowser = true;
  disableAudio(owner.stream);
  voice.rawMicStream?.getTracks().forEach((track) => track.stop());
  owner.dispose();
  voice.activeMicGain = null;
  voice.localStream = null;
  voice.rawMicStream = null;
  if (lost) {
    voice.micFailure = "unavailable";
    syncMicState();
    noteAudioProcessing({
      ...owner.info,
      actual: null,
      message: "Mikrofon getrennt; Standardgerät wird aktiviert",
      contextState: "closed",
    });
    noteMicLost(lost);
  } else {
    noteAudioProcessing({
      ...owner.info,
      actual: null,
      message: "Audioprozessor ausgefallen; Ersatzmikrofon wird aktiviert",
      contextState: "closed",
    });
    voice.deps?.onError?.(
      new Error(
        "Audioprozessor ausgefallen; lokale Aufnahme beendet, Browser-Ersatz wird aktiviert",
      ),
    );
  }
  void enqueueAudioCommit(async () => {
    if (voice.seat.generation === generation && sender?.track === failedTrack)
      await replaceSenderTrack(sender, null).catch(() => undefined);
  }).then(() => {
    if (
      voice.seat.generation === generation &&
      useVoice.getState().status === "joined" &&
      !voice.activeMicGain
    )
      void refreshMic();
  });
}
async function acquireMic(
  getMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>,
  current: () => boolean,
): Promise<{ raw: MediaStream; processor: MicProcessor }> {
  const owned: { processor?: MicProcessor } = {};
  const result = await captureMicrophone(
    getMedia,
    useMediaSettings.getState(),
    voice.micForceBrowser,
    () => {
      if (owned.processor) failedMicProcessor(owned.processor);
    },
    {
      current,
      acquired(raw) {
        disableAudio(raw);
        voice.pendingMicRaw.add(raw);
      },
      discarded(raw) {
        voice.pendingMicRaw.delete(raw);
      },
    },
    (info) => {
      if (owned.processor && voice.activeMicGain === owned.processor)
        noteAudioProcessing(info);
    },
  );
  owned.processor = result.processor;
  const latest = useMediaSettings.getState();
  const gain =
    voice.micForceBrowser || result.processor.nativeFallback
      ? 1
      : latest.inputGain;
  if (result.processor.stream === result.raw && gain !== 1) {
    const previous = result.processor;
    try {
      result.processor = await createProcessor(
        result.raw,
        { ...latest, inputGain: gain },
        result.processor.info.actual ?? "browser",
        () => {
          if (owned.processor) failedMicProcessor(owned.processor);
        },
        (info) => {
          if (owned.processor && voice.activeMicGain === owned.processor)
            noteAudioProcessing(info);
        },
      );
      previous.dispose();
    } catch (error) {
      result.processor.dispose();
      stopTracks(result.raw);
      voice.pendingMicRaw.delete(result.raw);
      if (current() && !voice.micForceBrowser) {
        voice.micForceBrowser = true;
        return acquireMic(getMedia, current);
      }
      throw error;
    }
  } else result.processor.setGain(gain);
  owned.processor = result.processor;
  if (!current() || !result.processor.usable()) {
    result.processor.dispose();
    stopTracks(result.raw);
    voice.pendingMicRaw.delete(result.raw);
    if (current() && !voice.micForceBrowser) {
      voice.micForceBrowser = true;
      return acquireMic(getMedia, current);
    }
    throw new Error(
      "Mikrofonanfrage abgebrochen oder Audioprozessor ausgefallen",
    );
  }
  voice.captureEpochs.set(result.processor.stream, captureEpoch(result.raw));
  return result;
}

function discardFailedMicCandidate(
  raw: MediaStream,
  insert: MicProcessor,
  current: boolean,
): void {
  insert.dispose();
  stopTracks(raw);
  voice.pendingMicRaw.delete(raw);
  if (current && !voice.micForceBrowser) {
    voice.micForceBrowser = true;
    voice.deps?.onError?.(
      new Error(
        "Audioprozessor vor Mikrofonwechsel ausgefallen; Browser-Ersatz wird aktiviert",
      ),
    );
    void refreshMic();
  }
}

/** A refused or blocked microphone: the user's own answer, asked again
 * only when they ask for it. */
function micRefused(error: unknown): boolean {
  const name = (error as { name?: unknown } | null)?.name;
  return name === "NotAllowedError" || name === "SecurityError";
}

/** The browser's reason stays in the console; the toast says it in German. */
function logMicFailure(error: unknown): void {
  logVoice("warn", "microphone", {
    detail:
      error instanceof Error
        ? `${error.name}: ${error.message}`
        : String(error),
  });
}

/** A join that goes on without a microphone. */
function reportJoinMicFailure(error: unknown): void {
  logMicFailure(error);
  voice.micFailure = micRefused(error) ? "denied" : "unavailable";
  syncMicState();
  voice.deps?.onError?.(microphoneFailure(error));
  noteAudioProcessing(null);
  const info = useAudioProcessing.getState();
  noteAudioProcessing({
    ...info,
    requested: useMediaSettings.getState().processingMode,
    message: "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu",
  });
}

/** Capture again for the connection that replaced the one a microphone
 * prompt was answered for, or have the next connection do it. Only within
 * the same join: a new join asks for its own microphone. A refused prompt is
 * not asked again but reported once. */
function resumeOvertakenMic(join: number, error?: unknown): void {
  if (
    join !== voice.micJoin ||
    useVoice.getState().status !== "joined" ||
    hasLiveTrack(voice.localStream, "audio")
  )
    return;
  if (micRefused(error)) {
    voice.micOvertaken = false;
    reportJoinMicFailure(error);
    return;
  }
  const pc = voice.seat.connection;
  if (pc && voice.startedConnections.has(pc) && voice.seat.isOpen()) {
    voice.micOvertaken = false;
    void refreshMic();
  } else voice.micOvertaken = true;
}

function announceMicrophone(): void {
  if (voice.announced.has("a")) return;
  const { serverId, channelId } = useVoice.getState();
  if (!serverId || !channelId) return;
  const self = currentUserId();
  if (self) setPub(self, "a", true);
  voice.announced.add("a");
  voice.deps?.gateway.send({
    op: "sig",
    t: "p",
    s: serverId,
    c: channelId,
    k: "a",
  });
}

/** A capture that now sends: the call has its microphone again, and watches
 * for losing it. */
function noteMicLive(raw: MediaStream): void {
  voice.micFailure = null;
  syncMicState();
  for (const track of raw.getAudioTracks())
    track.addEventListener("ended", () => onMicEnded(raw));
}

/** The microphone was unplugged or failed mid-call, where no processor
 * watches the capture: capture again, which takes the system default while
 * the chosen device is gone. */
function onMicEnded(raw: MediaStream): void {
  if (voice.rawMicStream !== raw || useVoice.getState().status !== "joined")
    return;
  voice.micFailure = "unavailable";
  syncMicState();
  fallBackToDefaultMic(raw);
}

function fallBackToDefaultMic(raw: MediaStream): void {
  if (voice.micLossHandled.has(raw)) return;
  noteMicLost(raw);
  void refreshMic();
}

/** Says once per capture that it is gone, and remembers to switch back. */
function noteMicLost(raw: MediaStream): void {
  if (voice.micLossHandled.has(raw)) return;
  voice.micLossHandled.add(raw);
  logVoice("warn", "microphone-lost");
  noteDiagnosticEvent({
    kind: "device-change",
    connection: "voice",
    detail: "audio-input-lost",
  });
  voice.micFellBack = useMediaSettings.getState().audioInputId !== "";
  voice.deps?.onError?.(
    new Error("Mikrofon getrennt – Standardgerät wird verwendet"),
  );
}

/** A device came or went. The chosen microphone gone: fall back as if it
 * had ended (some browsers keep the track). There (again, or for the first
 * time since joining without it): use it. */
async function onDeviceChange(): Promise<void> {
  const chosen = useMediaSettings.getState().audioInputId;
  if (useVoice.getState().status !== "joined" || !chosen || !voice.rawMicStream)
    return;
  const raw = voice.rawMicStream;
  let inputs: string[];
  try {
    inputs = (await navigator.mediaDevices.enumerateDevices())
      .filter((device) => device.kind === "audioinput")
      .map((device) => device.deviceId);
  } catch {
    return;
  }
  // Without permission the list has no ids and says nothing about the chosen one.
  if (
    inputs.every((id) => id === "") ||
    voice.rawMicStream !== raw ||
    useMediaSettings.getState().audioInputId !== chosen
  )
    return;
  const present = inputs.includes(chosen);
  // The capture names its device where the browser says: a chosen one that
  // was missing at joining is no loss when another device comes or goes.
  const using = raw.getAudioTracks()[0]?.getSettings?.().deviceId;
  const onChosen = using ? using === chosen : !voice.micFellBack;
  if (!present && onChosen) fallBackToDefaultMic(raw);
  else if (present && !onChosen) {
    voice.micFellBack = false;
    void refreshMic();
  }
}

/** Puts away the note of a call whose connection was lost. */
export function dismissLostCall(): void {
  if (useVoice.getState().status === "joined") return;
  useVoice.setState({ connection: "idle", lostCall: null });
}

/** The user asks for the microphone again after it was missing or refused. */
export function retryMicrophone(): void {
  if (useVoice.getState().status !== "joined") return;
  void refreshMic();
}

async function refreshMic(): Promise<void> {
  const pc = voice.seat.connection;
  if (!pc || useVoice.getState().status !== "joined") return;
  const session = voice.seat.generation;
  const join = voice.micJoin;
  const request = ++voice.micEpoch;
  const getUserMedia = voice.deps?.getUserMedia ?? defaultGetUserMedia;
  let candidate: Awaited<ReturnType<typeof acquireMic>>;
  try {
    candidate = await acquireMic(getUserMedia, () =>
      micCurrent(pc, session, request),
    );
  } catch (error) {
    // A track still sending stays until a new one replaces it. Without one
    // (a failed processor took it off the sender, or a rebuild asks again)
    // the call goes on without a microphone.
    if (micCurrent(pc, session, request)) {
      if (hasLiveTrack(voice.localStream, "audio")) {
        logMicFailure(error);
        voice.deps?.onError?.(
          microphoneFailure(error, "der Wechsel hat nicht geklappt"),
        );
      } else reportJoinMicFailure(error);
    } else if (voice.seat.generation !== session)
      resumeOvertakenMic(join, error);
    return;
  }
  const { raw, processor: insert } = candidate;
  const send = insert.stream;
  if (!micCurrent(pc, session, request)) {
    insert.dispose();
    stopTracks(raw);
    if (voice.seat.generation !== session) resumeOvertakenMic(join);
    return;
  }
  disableAudio(raw);
  voice.pendingMicRaw.add(raw);
  disableAudio(send);
  await enqueueAudioCommit(async () => {
    if (!micCurrent(pc, session, request)) {
      insert?.dispose();
      stopTracks(raw);
      voice.pendingMicRaw.delete(raw);
      if (voice.seat.generation !== session) resumeOvertakenMic(join);
      return;
    }
    if (!insert.usable()) {
      discardFailedMicCandidate(raw, insert, true);
      return;
    }
    const sender = audioSender();
    const previous = sender?.track ?? null;
    let outcome: "replaced" | "added" | "none";
    try {
      outcome = await commitMicSend(pc, send, session, raw);
    } catch (error) {
      insert?.dispose();
      stopTracks(raw);
      voice.pendingMicRaw.delete(raw);
      if (voice.seat.generation !== session) {
        // A rebuild closed this connection while the publish was in flight
        // (socket still open, e.g. an "unavailable" answer or ICE recovery).
        resumeOvertakenMic(join);
      } else if (
        micCurrent(pc, session, request) &&
        !hasLiveTrack(voice.localStream, "audio") &&
        !voice.seat.isOpen()
      ) {
        // The media socket just closed and its rebuild has not started yet:
        // that one asks for the microphone again.
        voice.micOvertaken = true;
      } else if (micCurrent(pc, session, request)) {
        voice.deps?.onError?.(
          error instanceof Error
            ? error
            : new Error(
                "Mikrofonwechsel fehlgeschlagen; bisheriges Mikrofon bleibt aktiv",
              ),
        );
      }
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      if (voice.seat.generation === session && voice.seat.connection === pc) {
        try {
          if (outcome === "replaced" && previous && voice.localStream) {
            await commitMicSend(pc, voice.localStream, session);
            applyLocalAudio();
          } else if (outcome === "added") await pc.closeSource("a");
        } catch {
          // seat.connection may already be tearing down
        }
      }
      discardFailedMicCandidate(raw, insert, micCurrent(pc, session, request));
      return;
    }
    const oldRaw = voice.rawMicStream;
    const oldSend = voice.localStream;
    const oldInsert = voice.activeMicGain;
    voice.rawMicStream = raw;
    voice.localStream = send;
    voice.activeMicGain = insert;
    noteMicLive(raw);
    noteAudioProcessing(insert.info);
    voice.pendingMicRaw.delete(raw);
    applyLocalAudio();
    announceMicrophone();
    try {
      await applySendBitrate();
    } catch {
      // bitrate is best-effort
    }
    oldInsert?.dispose();
    if (oldSend && oldSend !== oldRaw && oldSend !== send) {
      stopTracks(oldSend);
    }
    if (oldRaw && oldRaw !== raw) {
      stopTracks(oldRaw);
    }
  });
}

async function refreshCamera(): Promise<void> {
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

async function applyInputGain(): Promise<void> {
  await enqueueAudioCommit(async () => {
    if (
      !voice.rawMicStream ||
      useVoice.getState().status !== "joined" ||
      !voice.seat.connection
    )
      return;
    const settings = useMediaSettings.getState();
    if (
      voice.activeMicGain &&
      voice.activeMicGain.stream !== voice.rawMicStream
    ) {
      voice.activeMicGain.setGain(settings.inputGain);
      return;
    }
    if (settings.inputGain === 1) return;
    const pc = voice.seat.connection,
      raw = voice.rawMicStream,
      session = voice.seat.generation,
      request = voice.micEpoch;
    const actual = voice.activeMicGain?.info?.actual ?? "browser";
    let insert: MicProcessor | undefined;
    try {
      insert = await createProcessor(
        raw,
        settings,
        actual,
        () => {
          if (insert) failedMicProcessor(insert);
        },
        (info) => {
          if (voice.activeMicGain === insert) noteAudioProcessing(info);
        },
      );
    } catch (error) {
      voice.deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      insert.dispose();
      if (micCurrent(pc, session, request))
        voice.deps?.onError?.(
          new Error(
            "Mic-Gain konnte nicht aktiviert werden; bisheriges Mikrofon bleibt aktiv",
          ),
        );
      return;
    }
    disableAudio(insert.stream);
    const sender = audioSender(),
      previous = sender?.track ?? null;
    try {
      await commitMicSend(pc, insert.stream, session);
    } catch (error) {
      insert.dispose();
      voice.deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      if (voice.seat.connection === pc && voice.seat.generation === session)
        await replaceSenderTrack(sender, previous).catch(() => undefined);
      insert.dispose();
      if (micCurrent(pc, session, request))
        voice.deps?.onError?.(
          new Error("Mic-Gain ausgefallen; bisheriges Mikrofon bleibt aktiv"),
        );
      return;
    }
    voice.activeMicGain?.dispose();
    voice.activeMicGain = insert;
    voice.localStream = insert.stream;
    noteAudioProcessing(insert.info);
    applyLocalAudio();
  });
}

function queueInputGain(): void {
  void applyInputGain();
}

function audioSender(): RtpSender | undefined {
  return voice.seat.connection?.sender("a");
}

function cameraSender(): RtpSender | undefined {
  const cam = voice.cameraStream?.getVideoTracks()[0];
  if (!cam) return undefined;
  return voice.seat.connection
    ?.senders()
    .find((sender) => sender.track === cam);
}

function handleSettingsChange(prev: MediaSettings, next: MediaSettings): void {
  syncPushToTalk();
  applyLocalAudio();
  const joined = useVoice.getState().status === "joined";
  const recaptureMic =
    prev.audioInputId !== next.audioInputId ||
    prev.echoCancellation !== next.echoCancellation ||
    prev.noiseSuppression !== next.noiseSuppression ||
    prev.autoGainControl !== next.autoGainControl ||
    prev.processingMode !== next.processingMode;
  if (recaptureMic) voice.micForceBrowser = false;
  if (prev.audioInputId !== next.audioInputId) voice.micFellBack = false;
  const camChanged = prev.videoInputId !== next.videoInputId;
  const qualityChanged =
    prev.quality !== next.quality ||
    prev.economyMode !== next.economyMode ||
    prev.processingMode !== next.processingMode;
  if (joined) {
    const connection = "voice";
    if (prev.audioInputId !== next.audioInputId) {
      noteDiagnosticEvent({
        kind: "device-change",
        connection,
        detail: "audio-input",
      });
    }
    if (prev.audioOutputId !== next.audioOutputId) {
      noteDiagnosticEvent({
        kind: "device-change",
        connection,
        detail: "audio-output",
      });
    }
    if (prev.videoInputId !== next.videoInputId) {
      noteDiagnosticEvent({
        kind: "device-change",
        connection,
        detail: "video-input",
      });
    }
  }
  const cameraProfileChanged = prev.cameraProfile !== next.cameraProfile;
  const screenProfileChanged = prev.screenProfile !== next.screenProfile;
  if (joined && recaptureMic) void refreshMic();
  else if (joined && prev.inputGain !== next.inputGain) queueInputGain();
  if (joined && camChanged && useVoice.getState().camera) void refreshCamera();
  if (cameraProfileChanged) void applyStreamProfile("camera");
  if (screenProfileChanged) void applyStreamProfile("screen");
  if (
    joined &&
    (prev.videoUploadLimit !== next.videoUploadLimit ||
      prev.economyMode !== next.economyMode) &&
    voice.seat.connection
  ) {
    void voice.videoBudget.enqueue(voice.seat.connection);
  }
  if (joined && qualityChanged) {
    void applySendBitrate();
    void refreshAudioCodecs();
  }
  const sourceAppChanged = prev.sourceAudioApp !== next.sourceAudioApp;
  if (sharesSourceAudio(prev) !== sharesSourceAudio(next) || sourceAppChanged)
    applySourceAudioChoice(sourceAppChanged);
}

async function refreshAudioCodecs(): Promise<void> {
  const connection = publishingConnection(),
    mine = voice.seat.generation;
  if (!connection) return;
  try {
    if (voice.localStream?.getAudioTracks()[0])
      await commitMicSend(connection, voice.localStream, mine);
    if (voice.seat.connection !== connection || voice.seat.generation !== mine)
      return;
    if (voice.screenStream) await publishLocal("s", voice.screenStream);
    if (voice.seat.connection === connection && voice.seat.generation === mine)
      applyLocalAudio();
  } catch (error) {
    if (voice.seat.generation === mine) voice.deps?.onError?.(error);
  }
}

function occupySelf(state: {
  serverId: string | null;
  channelId: string | null;
  muted: boolean;
  deafened: boolean;
}): void {
  const self = currentUserId();
  if (!self || !state.serverId || !state.channelId) return;
  applyVoiceJoin(state.serverId, self, state.channelId, {
    muted: state.muted,
    deafened: state.deafened,
  });
}

function vacateSelf(serverId: string | null, channelId: string | null): void {
  const self = currentUserId();
  if (!self || !serverId || !channelId) return;
  applyVoiceLeave(serverId, self);
}

function sendFlag(
  kind: "m" | "d",
  serverId: string,
  channelId: string,
  on: boolean,
): void {
  voice.deps?.gateway.send({
    op: "sig",
    t: kind,
    s: serverId,
    c: channelId,
    on,
  });
}

function sendPub(kind: TrackKind, on: boolean): void {
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

/**
 * The media attempt failed, but the voice seat stays. Drop only the pubs
 * this attempt already announced, and release the camera and a Live.
 */
function retractAnnouncedMedia(): void {
  const state = useVoice.getState();
  const kinds = [...voice.announced];
  voice.announced.clear();
  const self = currentUserId();
  for (const kind of kinds) {
    if (self) setPub(self, kind, false);
    if (!state.serverId || !state.channelId) continue;
    voice.deps?.gateway.send({
      op: "sig",
      t: "u",
      s: state.serverId,
      c: state.channelId,
      k: kind,
    });
  }
  useVoice.setState({ camera: false, sharing: false });
}

/**
 * The gateway dropped pubs on detach. The tracks are still on the peer
 * connection, so announce them again. No new capture.
 */
function reannounceActive(): void {
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) return;
  const senders = voice.seat.connection?.senders() ?? [];
  if (senders.some((sender) => sender.track?.kind === "audio")) {
    if (hasLiveTrack(voice.localStream, "audio")) sendPub("a", true);
  }
  if (state.camera && voice.cameraStream) sendPub("v", true);
  if (state.sharing && voice.screenStream) {
    sendPub("s", true);
    if (hasLiveTrack(voice.screenStream, "audio")) sendPub("sa", true);
  }
}

const STREAM_TOAST =
  "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv.";
function reportStreamOnce(detail?: string): void {
  logVoice("warn", "stream", { detail });
  if (voice.streamReported) return;
  voice.streamReported = true;
  voice.deps?.onError?.(new Error(STREAM_TOAST));
}
function onTransportState(
  direction: Direction,
  state: string,
  mine: number,
): void {
  const peer = voice.seat;
  const slot = voice.seatRecoverySlot;
  if (peer.generation !== mine) return;
  if (state === "connected") {
    const timer = slot.get(direction);
    if (timer) clearTimeout(timer);
    slot.delete(direction);
    voice.seatRetry.cancel();
    if (slot.size === 0) setConnection("connected");
    noteDiagnosticEvent({
      kind: "recovery",
      connection: "voice",
      detail: `${direction}:connected`,
    });
    return;
  }
  if (state !== "failed" && state !== "disconnected") return;
  noteDiagnosticEvent({
    kind: "ice-error",
    connection: "voice",
    detail: `${direction}:${state}`,
  });
  setConnection("reconnecting");
  if (slot.has(direction)) return;
  const rebuild = () => {
    if (peer.generation === mine) scheduleSeatRebuild();
  };
  const restart = () => {
    if (peer.generation !== mine || !peer.connection) return;
    if (peer.connection.transportState(direction) === "connected") {
      slot.delete(direction);
      return;
    }
    slot.set(direction, setTimeout(rebuild, ICE_RECOVERY_DEADLINE_MS));
    void peer.connection.restartIce(direction).catch((error) => {
      if (peer.generation === mine) {
        voice.deps?.onError?.(error);
        rebuild();
      }
    });
  };
  if (state === "failed") restart();
  else
    slot.set(direction, setTimeout(restart, ICE_DISCONNECTED_RESTART_DELAY_MS));
}
function onMediaFrame(frame: MediaServerFrame): void {
  if (frame.op === "result") return;
  if (frame.op !== "err") {
    voice.seat.connection?.handleEvent(frame);
    return;
  }
  logVoice("warn", "media-error", { code: frame.e });
  if (frame.e === "ice_failed") {
    noteDiagnosticEvent({
      kind: "ice-error",
      connection: "voice",
      detail: "ice_failed",
    });
    return;
  }
  if (frame.e === "update_required") {
    rollbackSeat(
      new Error(
        "Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten.",
      ),
    );
    return;
  }
  if (frame.e === "unauthorized") {
    rollbackSeat(
      new ApiError("unauthenticated", 0, errorMessage("unauthenticated")),
    );
    return;
  }
  if (frame.e === "unavailable") {
    if (voice.seatRetry.active) {
      stopPeer(true);
      scheduleSeatRebuild();
      return;
    }
    voice.deps?.onError?.(new Error("Kein freier Sprachplatz."));
    retractAnnouncedMedia();
    stopPeer();
    setConnection("failed");
    return;
  }
  if (frame.e === "forbidden") {
    voice.deps?.onError?.(
      new ApiError("forbidden", 0, errorMessage("forbidden")),
    );
    return;
  }
  if (!voice.seat.accepted) {
    rollbackSeat(new MediaError(frame.e, frame.lc));
    return;
  }
  reportStreamOnce(frame.e);
}

/**
 * Join a voice channel. Local state flips first so the click feels instant;
 * the socket frame, ticket, and ICE run after.
 */
export function joinVoice(input: {
  serverId: string;
  channelId: string;
  channelName: string;
}): void {
  installDiagnosticsLogoutReset();
  ensureBound();
  const userId = currentUserId();
  if (!userId) return;
  const prev = useVoice.getState();
  voice.soundOnJoin =
    prev.serverId !== input.serverId || prev.channelId !== input.channelId;
  unlockCallSounds();
  const muted = prev.status === "joined" && (prev.muted || prev.deafened);
  const deafened = prev.status === "joined" && prev.deafened;
  setCallSoundsDeafened(deafened);
  if (prev.status !== "joined") voice.preDeafenMuted = false;
  resetDiagnostics();
  voice.streamReported = false;
  if (
    prev.status === "joined" &&
    prev.serverId &&
    prev.channelId &&
    (prev.serverId !== input.serverId || prev.channelId !== input.channelId)
  ) {
    vacateSelf(prev.serverId, prev.channelId);
    voice.deps?.gateway.send({
      op: "sig",
      t: "l",
      s: prev.serverId,
      c: prev.channelId,
    });
  }
  stopPeer();
  const self = userId;
  useVoice.setState({
    status: "joined",
    connection: "connecting",
    lostCall: null,
    serverId: input.serverId,
    channelId: input.channelId,
    channelName: input.channelName,
    muted,
    deafened,
    camera: false,
    sharing: false,
    localCamera: null,
    localScreen: null,
    sourceSubscriptions: {},
    sourceWatchSupported: false,
    remote: {},
    participants: { [self]: { pubs: [] } },
  });
  syncMicState();
  applyVoiceJoin(input.serverId, self, input.channelId, {
    muted,
    deafened,
  });
  voice.awaitingJoin = { serverId: input.serverId, channelId: input.channelId };
  voice.deps?.gateway.send({
    op: "sig",
    t: "j",
    s: input.serverId,
    c: input.channelId,
  });
  // The join frame has no mute fields. Sync flags in socket order before media work.
  if (muted) sendFlag("m", input.serverId, input.channelId, true);
  if (deafened) sendFlag("d", input.serverId, input.channelId, true);
  voice.receivedAudio.applyPlayback();
  void startPeer(input.serverId, input.channelId);
}

export function leaveVoice(): void {
  const state = useVoice.getState();
  const hadJoined = !voice.awaitingJoin && state.status === "joined";
  voice.soundOnJoin = false;
  stopCallSounds();
  if (hadJoined && !state.deafened) playCallSound("leave");
  setCallSoundsDeafened(false);
  const serverId = state.serverId;
  const channelId = state.channelId;
  voice.awaitingJoin = null;
  voice.republishOnJoin = false;
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...IDLE_VOICE,
    playbackBlocked: voice.receivedAudio.playbackBlocked,
  });
  voice.receivedAudio.applyPlayback();
  if (serverId && channelId) {
    voice.deps?.gateway.send({
      op: "sig",
      t: "l",
      s: serverId,
      c: channelId,
    });
  }
}

/**
 * Mute toggles immediately; the sig frame is the sync, not the wait.
 * Unmuting while deafened also undeafens so you can hear yourself speak.
 */
export function toggleMute(): void {
  ensureBound();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  const prevMuted = state.muted;
  const prevDeafened = state.deafened;
  const muted = !(state.muted || state.deafened);
  const deafened = muted ? state.deafened : false;
  if (!deafened) voice.preDeafenMuted = muted;
  useVoice.setState({ muted, deafened });
  setCallSoundsDeafened(deafened);
  playCallSound(muted ? "mute" : "unmute");
  applyLocalAudio();
  occupySelf({ ...state, muted, deafened });
  if (muted !== prevMuted) {
    sendFlag("m", state.serverId, state.channelId, muted);
  }
  if (deafened !== prevDeafened) {
    sendFlag("d", state.serverId, state.channelId, deafened);
  }
}

/**
 * Deafen toggles immediately. Turning it on also mutes; turning it off
 * restores the previous mute choice. The member list is updated before the server round-trip.
 */
export function toggleDeafen(): void {
  ensureBound();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  const prevMuted = state.muted;
  const prevDeafened = state.deafened;
  const deafened = !state.deafened;
  if (deafened) voice.preDeafenMuted = state.muted;
  const muted = deafened || voice.preDeafenMuted;
  useVoice.setState({ muted, deafened });
  setCallSoundsDeafened(deafened);
  playCallSound(deafened ? "deafen" : "undeafen");
  applyLocalAudio();
  occupySelf({ ...state, muted, deafened });
  if (muted !== prevMuted) {
    sendFlag("m", state.serverId, state.channelId, muted);
  }
  if (deafened !== prevDeafened) {
    sendFlag("d", state.serverId, state.channelId, deafened);
  }
}

/**
 * Camera preview is local and immediate; publish rides the media path.
 */
export function toggleCamera(): void {
  ensureBound();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  if (state.camera) {
    stopLocalVideo("v");
    return;
  }
  useVoice.setState({ camera: true });
  void startLocalVideo("v");
}

/**
 * Screen-share preview is local; SDP stays off the chat socket so the
 * message pane does not stutter when share starts.
 */
export function toggleShare(): void {
  ensureBound();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  if (state.sharing) {
    stopLocalVideo("s");
    return;
  }
  useVoice.setState({ sharing: true });
  void startLocalVideo("s");
}

export function resetVoiceForTests(): void {
  stopCallSounds();
  setCallSoundsDeafened(false);
  resetDiagnostics();
  clearSeatReconnectTimer();
  stopPeer();
  resetPlaybackBoostForTests();
  const previous = voice;
  voice = createVoiceSession(previous);
  useVoice.setState({ ...IDLE_VOICE });
  onMediaSettingsChange(null);
  previous.unsubscribeUserVolumes?.();
  previous.unsubscribePushToTalk?.();
  onPushToTalkGate(null);
  configurePushToTalk(null);
}

/** The module's state, for tests: what a reset leaves behind. */
export function voiceSessionForTests(): {
  current: Readonly<VoiceSession>;
  fresh: Readonly<VoiceSession>;
  epochs: readonly string[];
} {
  return { current: voice, fresh: createVoiceSession(), epochs: VOICE_EPOCHS };
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

function bumpVideoEpoch(kind: LocalVideo): number {
  return kind === "v" ? ++voice.cameraEpoch : ++voice.screenEpoch;
}

/** Stopping bumps the epoch, so an abandoned start is not open. A rebuild of
 * the seat's connection leaves an open start its epoch. */
function captureOpen(kind: LocalVideo): boolean {
  return voice.openCaptures.get(kind) === videoEpoch(kind);
}

/** The browser's own getUserMedia reasons are English ("Requested device not
 * found"); say them in German with what follows for the call: without a
 * sending microphone it goes on without one, a failed switch keeps the
 * previous one. */
function microphoneFailure(
  error: unknown,
  consequence = "du hörst nur zu",
): unknown {
  switch ((error as { name?: unknown } | null)?.name) {
    case "NotFoundError":
    case "OverconstrainedError":
      return new Error(`Kein Mikrofon gefunden, ${consequence}.`);
    case "NotAllowedError":
    case "SecurityError":
      return new Error(`Mikrofonzugriff nicht erlaubt, ${consequence}.`);
    case "NotReadableError":
    case "AbortError":
      return new Error(
        `Das Mikrofon ist gerade nicht verfügbar, ${consequence}.`,
      );
    default:
      return error;
  }
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

async function startLocalVideo(kind: LocalVideo): Promise<void> {
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

function stopLocalVideo(kind: LocalVideo): void {
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

async function publishLocal(
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

/** The own Live's sound. */
function setSourceAudio(
  status: SourceAudioStatus,
  note: SourceAudioNote | null = null,
): void {
  useVoice.setState({
    sourceAudio: { s: status },
    sourceAudioNote: { s: note },
  });
}

function bindSourceAudio(stream: MediaStream, track: MediaStreamTrack): void {
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
async function captureNativeSourceAudio(
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
function applySourceAudioChoice(appChanged: boolean): void {
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

/** A user's source subscription survives route changes and transport recovery. */
export function toggleSourceWatch(userId: string, kind: "s" | "l"): void {
  const state = useVoice.getState();
  if (state.status !== "joined" || userId === currentUserId()) return;
  const on = !state.sourceSubscriptions[userId]?.[kind];
  useVoice.setState({
    sourceSubscriptions: {
      ...state.sourceSubscriptions,
      [userId]: { ...state.sourceSubscriptions[userId], [kind]: on },
    },
  });
  if (state.sourceWatchSupported)
    void voice.seat
      .request("w", { u: userId, k: kind, on })
      .catch((error) => voice.deps?.onError?.(error));
  if (!on) {
    dropRemote(userId, kind);
    voice.receivedAudio.clearSourcePlayback(userId, kind);
  } else {
    reattachReceived(userId, kind);
    voice.receivedAudio.resumeSource(userId, kind);
  }
}

function parseIncomingVideo(
  track: MediaStreamTrack,
): { userId: string; k: "v" | "s" | "l" } | null {
  const identity = voice.receivedIdentity.get(track);
  const parsed = identity ? { userId: identity.owner, k: identity.k } : null;
  return parsed && (parsed.k === "v" || parsed.k === "s" || parsed.k === "l")
    ? { userId: parsed.userId, k: parsed.k }
    : null;
}

function attachIncoming(track: MediaStreamTrack, stream?: MediaStream): void {
  if (track.kind === "audio") {
    voice.receivedAudio.attachIncoming(track, stream);
    return;
  }
  const parsed = parseIncomingVideo(track);
  if (!parsed) {
    logVoice("warn", "track", { track: track.kind, detail: "untagged" });
    return;
  }
  logVoice("info", "track", { track: parsed.k, peer: parsed.userId });
  const attached = stream ?? createStream([track]);
  if (!attached) return;
  noteReceived(parsed.userId, parsed.k, attached, track);
  const state = useVoice.getState();
  if (
    parsed.k !== "v" &&
    state.sourceWatchSupported &&
    !state.sourceSubscriptions[parsed.userId]?.[parsed.k]
  )
    return;
  const current = state.remote[parsed.userId] ?? {};
  useVoice.setState({
    remote: {
      ...state.remote,
      [parsed.userId]: { ...current, [parsed.k]: attached },
    },
  });
}

function scheduleSeatRebuild(): void {
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) return;
  setConnection("reconnecting");
  const mine = voice.seat.generation;
  voice.seatRetry.schedule(
    () => {
      const current = useVoice.getState();
      if (
        voice.seat.generation !== mine ||
        current.serverId !== state.serverId ||
        current.channelId !== state.channelId ||
        current.status !== "joined"
      )
        return;
      void startPeer(state.serverId!, state.channelId!, true);
    },
    () =>
      rollbackSeat(
        new Error(
          "Die Sprachverbindung konnte nicht wiederhergestellt werden. Bitte erneut beitreten.",
        ),
        true,
      ),
  );
}

/** The browser SDK's chunk did not arrive (`sdk_load_*`); `again` says how
 * to retry. A tab whose chunk a deploy replaced only recovers by reloading. */
function sdkLoadError(error: unknown, again: string): Error | null {
  if (!(error instanceof MediaError)) return null;
  if (error.code === "sdk_load_failed")
    return new Error(
      `Die Anruffunktion konnte nicht geladen werden. Bitte Gelabber neu laden und ${again}`,
    );
  if (error.code === "sdk_load_timeout")
    return new Error(
      `Die Anruffunktion lädt gerade sehr langsam. Bitte gleich ${again}`,
    );
  return null;
}

function retryableTicketError(error: unknown): boolean {
  return (
    !(error instanceof ApiError) ||
    ["network", "timeout", "internal", "rate_limited"].includes(error.code)
  );
}

async function startPeer(
  serverId: string,
  channelId: string,
  recovering = false,
): Promise<void> {
  const mine = voice.seat.generation + 1;
  const resumeCamera = useVoice.getState().camera;
  const resumeShare = useVoice.getState().sharing;
  stopPeer(recovering);
  voice.seat.generation = mine;
  const join = voice.micJoin;
  if (!recovering) {
    if (resumeCamera) useVoice.setState({ camera: true });
    if (resumeShare) useVoice.setState({ sharing: true });
  }
  const fetchTicket = voice.deps?.fetchTicket ?? requestMediaTicket;
  const openMedia = voice.deps?.openMedia ?? openMediaSocket;
  const getUserMedia = voice.deps?.getUserMedia ?? defaultGetUserMedia;
  let pc: MediaConnection;
  if (!voice.deps?.createMediaConnection) preloadMediaSdk();
  try {
    const ticket = await fetchTicket(channelId);
    if (voice.seat.generation !== mine) return;
    const socket = openMedia(mediaWsUrl(ticket.media_path));
    voice.seat.bind(
      socket,
      (frame) => {
        if (voice.seat.generation === mine) onMediaFrame(frame);
      },
      () => {
        if (
          voice.seat.generation === mine &&
          useVoice.getState().status === "joined"
        )
          scheduleSeatRebuild();
      },
    );
    const joined = await voice.seat.request("j", {
      tk: ticket.ticket,
      v: MEDIA_VERSION,
    });
    if (voice.seat.generation !== mine) return;
    pc = makeMediaConnection(
      voice.seat,
      joined.generation,
      ticket.ice_servers ?? [],
      mine,
    );
    await pc.start(joined.routerRtpCapabilities);
    if (voice.seat.generation !== mine || voice.seat.connection !== pc) return;
    voice.startedConnections.add(pc);
    setConnection("connected");
    useVoice.setState({ sourceWatchSupported: true });
    for (const [userId, subscriptions] of Object.entries(
      useVoice.getState().sourceSubscriptions,
    ))
      for (const kind of ["s", "l"] as const)
        if (subscriptions[kind])
          await voice.seat.request("w", { u: userId, k: kind, on: true });
    attachSeatDiagnostics(mine);
  } catch (error) {
    if (voice.seat.generation !== mine) return;
    if (error instanceof MediaError && error.code === "update_required") {
      rollbackSeat(
        new Error(
          "Bitte Gelabber aktualisieren und den Sprachkanal erneut betreten.",
        ),
      );
      return;
    }
    const unloaded = sdkLoadError(error, "den Sprachkanal erneut betreten.");
    if (unloaded) {
      rollbackSeat(unloaded);
      return;
    }
    if (recovering && retryableTicketError(error)) {
      stopPeer(true);
      scheduleSeatRebuild();
      return;
    }
    rollbackSeat(error, retryableTicketError(error));
    return;
  }

  if (hasLiveTrack(voice.localStream, "audio")) {
    try {
      await commitMicSend(pc, voice.localStream!, mine);
    } catch (error) {
      if (voice.seat.generation === mine && voice.seat.connection === pc) {
        voice.deps?.onError?.(error);
        stopPeer(true);
        scheduleSeatRebuild();
      }
      return;
    }
    applyLocalAudio();
    voice.announced.add("a");
    voice.deps?.gateway.send({
      op: "sig",
      t: "p",
      s: serverId,
      c: channelId,
      k: "a",
    });
    await applySendBitrate().catch(() => undefined);
  } else if (recovering && voice.micOvertaken) {
    // A rebuild does not wait for a prompt: what it carried goes out first.
    voice.micOvertaken = false;
    void refreshMic();
  } else if (!recovering) {
    const micRequest = ++voice.micEpoch;
    try {
      const { raw: stream, processor: insert } = await acquireMic(
        getUserMedia,
        () =>
          voice.seat.generation === mine &&
          voice.micEpoch === micRequest &&
          voice.seat.connection === pc,
      );
      const send = insert.stream;
      if (voice.seat.generation !== mine) {
        insert.dispose();
        stopTracks(stream);
        resumeOvertakenMic(join);
        return;
      }
      if (voice.micEpoch !== micRequest) {
        insert.dispose();
        stopTracks(stream);
        // A newer refreshMic owns capture; continue without this stream.
      } else {
        disableAudio(stream);
        voice.pendingMicRaw.add(stream);
        disableAudio(send);
        await enqueueAudioCommit(async () => {
          if (
            voice.seat.generation !== mine ||
            voice.micEpoch !== micRequest ||
            voice.seat.connection !== pc
          ) {
            insert?.dispose();
            stopTracks(stream);
            voice.pendingMicRaw.delete(stream);
            if (voice.seat.generation !== mine) resumeOvertakenMic(join);
            return;
          }
          if (!insert.usable()) {
            discardFailedMicCandidate(stream, insert, true);
            return;
          }
          voice.rawMicStream = stream;
          voice.localStream = send;
          voice.activeMicGain = insert;
          noteMicLive(stream);
          noteAudioProcessing(insert.info);
          voice.pendingMicRaw.delete(stream);
          send
            .getAudioTracks()
            .forEach((track) =>
              hintTrack(
                track,
                useMediaSettings.getState().processingMode === "original"
                  ? "music"
                  : "speech",
              ),
            );
          applyLocalAudio();
          await commitMicSend(pc, send, mine);
          applyLocalAudio();
          try {
            await applySendBitrate();
          } catch {
            // bitrate is best-effort
          }
          if (
            voice.seat.generation !== mine ||
            voice.activeMicGain !== insert ||
            !insert.usable()
          )
            return;
          announceMicrophone();
        });
      }
    } catch (error) {
      if (voice.seat.generation !== mine) resumeOvertakenMic(join, error);
      else reportJoinMicFailure(error);
    }
  }

  if (voice.seat.generation !== mine) return;

  // What the connection before this one carried, and what was captured
  // while this one could not publish yet.
  const pending = useVoice.getState();
  if (pending.localCamera) {
    await publishLocal("v", pending.localCamera);
  }
  if (pending.localScreen) {
    await publishLocal("s", pending.localScreen);
  }
  // A button pressed during the connect has opened its own picker or prompt,
  // and may have been switched off again while the publishes above ran.
  const wanted = useVoice.getState();
  if (
    !recovering &&
    (wanted.camera || resumeCamera) &&
    !wanted.localCamera &&
    !captureOpen("v")
  ) {
    void startLocalVideo("v");
  }
  if (
    !recovering &&
    (wanted.sharing || resumeShare) &&
    !wanted.localScreen &&
    !captureOpen("s")
  ) {
    void startLocalVideo("s");
  }
  if (voice.seat.generation !== mine) return;
}
