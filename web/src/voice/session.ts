// Local voice state, capture and playback; mediasoup owns browser transport.
// Join updates the store immediately; ticket / ICE / getUserMedia run after.
// Camera and Live (a screen): local preview first, publish on the media path.
// An older client's Go Live is received like a Live. Chat WS: presence
// (j/l/p/u).
// Own versioned Media WS: public mediasoup-client transport and source signaling.
// The parts of a call live beside it: sessionState.ts (the state every part
// shares), callMicrophone.ts, localVideo.ts, localSourceAudio.ts,
// receivedAudio.ts and videoLimits.ts.

import { noteAudioProcessing } from "./audioProcessing.ts";
import { readSenderPriority } from "./mediaPriority.ts";
import {
  playCallSound,
  unlockCallSounds,
  stopCallSounds,
  setCallSoundsDeafened,
} from "./callSounds.ts";
import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { notifyError } from "../components/toasts.ts";
import { getGateway } from "../ws/client.ts";
import type { ErrFrame, SigEvent, TrackKind } from "../ws/protocol.ts";
import { applyVoiceJoin, applyVoiceLeave } from "./roster.ts";
import {
  type IceServer,
  type MediaServerFrame,
  mediaWsUrl,
  openMediaSocket,
  requestMediaTicket,
  MEDIA_VERSION,
  MediaError,
} from "./media.ts";
import { MediaPeer } from "./mediaPeer.ts";
import { createStream } from "./native/tracks.ts";
import { resetPlaybackBoostForTests } from "./playbackBoost.ts";
import { useUserVolumes } from "./userVolume.ts";
import {
  createMediaConnection,
  preloadMediaSdk,
  type MediaConnection,
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
  audioBitrate,
  sourceAudioBitrate,
  onMediaSettingsChange,
  sharesSourceAudio,
  streamProfileFps,
  useMediaSettings,
} from "./settings.ts";
import { configurePushToTalk, onPushToTalkGate } from "./pushToTalk.ts";
import {
  IDLE_VOICE,
  useVoice,
  type VoiceConnection,
  type VoiceParticipant,
  type VoiceState,
} from "./voiceStore.ts";
import { applySourceAudioChoice } from "./localSourceAudio.ts";
import {
  applyStreamProfile,
  bumpVideoEpoch,
  captureOpen,
  publishLocal,
  refreshCamera,
  startLocalVideo,
  stopLocalVideo,
} from "./localVideo.ts";
import {
  acquireMic,
  announceMicrophone,
  commitMicSend,
  disableAudio,
  discardFailedMicCandidate,
  disposeMicGain,
  enqueueAudioCommit,
  noteMicLive,
  onDeviceChange,
  queueInputGain,
  refreshMic,
  reportJoinMicFailure,
  resumeOvertakenMic,
} from "./callMicrophone.ts";
import {
  applyLocalAudio,
  applySendBitrate,
  createVoiceSession,
  currentUserId,
  renewVoiceSession,
  defaultGetDisplayMedia,
  defaultGetUserMedia,
  type Direction,
  hasLiveTrack,
  hintTrack,
  type LocalVideo,
  logVoice,
  profileForVideoTrack,
  publishingConnection,
  type RecoverySlot,
  reportStreamOnce,
  sendPub,
  setPub,
  stopTracks,
  syncMicState,
  voice,
  VOICE_EPOCHS,
  type VoiceDeps,
  type VoiceSession,
} from "./sessionState.ts";

export { retryMicrophone } from "./callMicrophone.ts";
export type { RtpSender, VoiceDeps, VoiceGateway } from "./sessionState.ts";
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

const LIVE_FORBIDDEN_MESSAGE = "In diesem Server darfst du nicht live gehen.";

const ICE_DISCONNECTED_RESTART_DELAY_MS = 2000;

const ICE_RECOVERY_DEADLINE_MS = 10_000;

function clearRecovery(slot: RecoverySlot): void {
  for (const timer of slot.values()) clearTimeout(timer);
  slot.clear();
}

function clearSeatReconnectTimer(): void {
  clearRecovery(voice.seatRecoverySlot);
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

/** Reports the connection; observed only, the retries do not read it. */
function setConnection(connection: VoiceConnection): void {
  if (useVoice.getState().connection !== connection)
    useVoice.setState({ connection });
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

/** Puts away the note of a call whose connection was lost. */
export function dismissLostCall(): void {
  if (useVoice.getState().status === "joined") return;
  useVoice.setState({ connection: "idle", lostCall: null });
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
  const previous = renewVoiceSession();
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
