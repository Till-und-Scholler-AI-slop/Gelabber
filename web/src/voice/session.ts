import {
  captureMicrophone,
  createProcessor,
  noteAudioProcessing,
  useAudioProcessing,
  type MicProcessor,
} from "./audioProcessing.ts";
import {
  playCallSound,
  unlockCallSounds,
  stopCallSounds,
  setCallSoundsDeafened,
} from "./callSounds.ts";
// Local voice state + native RTCPeerConnection.
// Join updates the store immediately; ticket / ICE / getUserMedia run after.
// Camera / screen / Go Live: local preview first, publish on the media path.
// Watch is subscribe-only — no getUserMedia. Chat WS: presence (j/l/p/u).
// Media WS: SDP/ICE + RTP. No product SDK.

import { create } from "zustand";

import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { useSession } from "../auth/session.ts";
import { notifyError } from "../components/toasts.ts";
import { getGateway, type Gateway } from "../ws/client.ts";
import type { ErrFrame, SigEvent, TrackKind } from "../ws/protocol.ts";
import {
  applyLiveEnd,
  applyLiveStart,
  applyVoiceJoin,
  applyVoiceLeave,
  liveOf,
  useVoiceRoster,
} from "./roster.ts";
import {
  type IceServer,
  type MediaServerFrame,
  type OpenMedia,
  mediaWsUrl,
  openMediaSocket,
  publishedTrackIds,
  requestMediaTicket,
  tuneAudioSdp,
} from "./media.ts";
import { type IceCand, MediaPeer, MediaRetry } from "./mediaPeer.ts";
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
  allocateVideoBitrates,
  audioBitrate,
  sourceAudioBitrate,
  SOURCE_AUDIO_CONSTRAINTS,
  isOverconstrainedError,
  noteStreamProfileApply,
  onMediaSettingsChange,
  streamProfileFps,
  useMediaSettings,
  videoConstraintLadder,
  videoConstraintsFor,
} from "./settings.ts";

type VideoKind = "v" | "s" | "l";
type SourceAudioKind = "sa" | "la";
type PublishKind = VideoKind | SourceAudioKind;
export type SourceAudioStatus =
  "off" | "sharing" | "unavailable" | "ended" | "unsupported";

export type VoiceStatus = "idle" | "joined";

export type VoiceParticipant = {
  pubs: TrackKind[];
};

export type RemoteVideo = {
  v?: MediaStream;
  s?: MediaStream;
  l?: MediaStream;
};

export type VoiceState = {
  status: VoiceStatus;
  serverId: string | null;
  channelId: string | null;
  channelName: string | null;
  muted: boolean;
  deafened: boolean;
  camera: boolean;
  sharing: boolean;
  live: boolean;
  sourceWatchSupported: boolean;
  sourceAudio: Record<"s" | "l", SourceAudioStatus>;
  sourceSubscriptions: Record<string, Partial<Record<"s" | "l", boolean>>>;
  localCamera: MediaStream | null;
  localScreen: MediaStream | null;
  localLive: MediaStream | null;
  watching: boolean;
  watchServerId: string | null;
  watchChannelId: string | null;
  watchChannelName: string | null;
  watchPublisherId: string | null;
  watchStream: MediaStream | null;
  playbackBlocked: boolean;
  remote: Record<string, RemoteVideo>;
  participants: Record<string, VoiceParticipant>;
};

const idle: VoiceState = {
  status: "idle",
  serverId: null,
  channelId: null,
  channelName: null,
  muted: false,
  deafened: false,
  camera: false,
  sharing: false,
  live: false,
  sourceWatchSupported: false,
  sourceAudio: { s: "off", l: "off" },
  sourceSubscriptions: {},
  localCamera: null,
  localScreen: null,
  localLive: null,
  watching: false,
  watchServerId: null,
  watchChannelId: null,
  watchChannelName: null,
  watchPublisherId: null,
  watchStream: null,
  playbackBlocked: false,
  remote: {},
  participants: {},
};

export const useVoice = create<VoiceState>(() => ({ ...idle }));

// Remember a deliberate mute while temporarily deafened, also across room switches.
let preDeafenMuted = false;

export type RtpEncodingParameters = {
  maxBitrate?: number;
  maxFramerate?: number;
};

export type RtpSenderParameters = {
  encodings: RtpEncodingParameters[];
  transactionId?: string;
};

export type RtpSender = {
  track: MediaStreamTrack | null;
  replaceTrack?(track: MediaStreamTrack | null): Promise<void>;
  getParameters?(): RtpSenderParameters;
  setParameters?(params: RtpSenderParameters): Promise<void>;
};

export type PeerConnection = {
  onicecandidate:
    | ((event: {
        candidate: { candidate: string; sdpMid: string | null } | null;
      }) => void)
    | null;
  ontrack:
    | ((event: { streams: MediaStream[]; track: MediaStreamTrack }) => void)
    | null;
  onnegotiationneeded: (() => void) | null;
  addTrack?(track: MediaStreamTrack, stream: MediaStream): RtpSender | void;
  addTransceiver?(
    kind: "audio" | "video",
    init?: { direction?: "recvonly" | "sendonly" | "sendrecv" | "inactive" },
  ): void;
  removeTrack?(sender: RtpSender): void;
  getSenders?(): RtpSender[];
  createOffer(options?: { iceRestart?: boolean }): Promise<{
    type: string;
    sdp?: string;
  }>;
  createAnswer(): Promise<{ type: string; sdp?: string }>;
  setLocalDescription(desc: { type: string; sdp?: string }): Promise<void>;
  setRemoteDescription(desc: { type: string; sdp?: string }): Promise<void>;
  getTransceivers?(): {
    mid?: string | null;
    direction?: "recvonly" | "sendonly" | "sendrecv" | "inactive";
    stopped?: boolean;
    sender?: { track?: { kind: string } | null };
    receiver?: { track?: { kind: string } | null };
    setCodecPreferences?(codecs: { mimeType: string }[]): void;
  }[];
  addIceCandidate(candidate: {
    candidate: string;
    sdpMid: string | null;
  }): Promise<void>;
  close(): void;
  iceConnectionState?: string;
  connectionState?: string;
  oniceconnectionstatechange: (() => void) | null;
  onconnectionstatechange: (() => void) | null;
  remoteDescription?: { type: string; sdp?: string } | null;
  localDescription?: { type: string; sdp?: string } | null;
  signalingState?: string;
  getStats?(): Promise<unknown>;
};

export type VoiceGateway = Pick<
  Gateway,
  "send" | "onSig" | "onErr" | "onReady"
>;

export type VoiceDeps = {
  gateway: VoiceGateway;
  userId: () => string | null;
  createPeer: (iceServers: IceServer[]) => PeerConnection;
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
  attachRemote?: (stream: MediaStream) => void;
  onError?: (error: unknown) => void;
};

type MicGainInsert = MicProcessor;
let micForceBrowser = false;

let deps: VoiceDeps | null = null;
/** Seat media peer. `joinVoice` / `leaveVoice` hold this object. */
let seat = new MediaPeer();
let localStream: MediaStream | null = null;
let rawMicStream: MediaStream | null = null;
let activeMicGain: MicGainInsert | null = null;
let cameraStream: MediaStream | null = null;
let screenStream: MediaStream | null = null;
let liveStream: MediaStream | null = null;
let seatMediaVersion: number | null = null;
let remoteAudio: HTMLAudioElement | null = null;
const watchAudio = new Map<MediaStreamTrack, HTMLAudioElement>();
const receivedAudioSources = new Map<
  MediaStreamTrack,
  { role: "voice" | "watch"; source: AudioSource }
>();
const receivedSourceAudio = new Map<
  MediaStreamTrack,
  { stream?: MediaStream; userId: string; kind: "s" | "l" }
>();
const sourceAudio = new Map<
  MediaStreamTrack,
  {
    el: HTMLAudioElement;
    role: "voice" | "watch";
    userId: string;
    kind: "s" | "l";
  }
>();
const blockedPlayback = new Set<HTMLAudioElement>();
let remoteMix: MediaStream | null = null;
let bound = false;
let micEpoch = 0;
/** One stream toast per join. Further failures stay in the console. */
let streamReported = false;
let watchReported = false;
let cameraEpoch = 0;
let screenEpoch = 0;
let cameraProfileEpoch = 0;
let screenProfileEpoch = 0;
let liveEpoch = 0;
let awaitingJoin: { serverId: string; channelId: string } | null = null;
let awaitingLive: { serverId: string; channelId: string } | null = null;
let liveClaimNonce: string | null = null;
let liveClaimTimer: ReturnType<typeof setTimeout> | null = null;
let withdrawnLiveNonce: string | null = null;
let liveRecoveryDeadline: number | null = null;
let watchPublisherTimer: ReturnType<typeof setTimeout> | null = null;

function isLiveNonce(value: string | undefined): value is string {
  return Boolean(
    value &&
    /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/.test(value) &&
    !/^0{8}-0{4}-0{4}-0{4}-0{12}$/.test(value),
  );
}

function clearWatchPublisherTimer(): void {
  if (watchPublisherTimer !== null) clearTimeout(watchPublisherTimer);
  watchPublisherTimer = null;
}

function clearLiveClaim(): void {
  if (liveClaimTimer !== null) clearTimeout(liveClaimTimer);
  liveClaimTimer = null;
  liveClaimNonce = null;
  withdrawnLiveNonce = null;
  liveRecoveryDeadline = null;
}

function requestLiveClaim(): void {
  const state = useVoice.getState();
  if (!state.serverId || !state.channelId || !state.live) return;
  if (liveClaimTimer !== null) clearTimeout(liveClaimTimer);
  liveClaimNonce = null;
  awaitingLive = { serverId: state.serverId, channelId: state.channelId };
  liveClaimTimer = setTimeout(
    () => {
      liveClaimTimer = null;
      if (!awaitingLive) return;
      awaitingLive = null;
      stopLocalVideo("l");
      deps?.onError?.(
        new Error("Live konnte nicht bestätigt werden. Bitte erneut starten."),
      );
    },
    Math.max(
      0,
      Math.min(
        10000,
        (liveRecoveryDeadline ?? Date.now() + 10000) - Date.now(),
      ),
    ),
  );
  sendPub("l", true);
}
let audioCommitChain: Promise<void> = Promise.resolve();
let cameraCommitChain: Promise<void> = Promise.resolve();
/**
 * One video-budget queue for the current peer generation. Profile changes,
 * publish, unpublish, and SDP all share it. A newer request supersedes an
 * in-flight snapshot so an older pass cannot write a stale sender cap.
 */
let videoLimitChain: Promise<void> = Promise.resolve();
let videoLimitGeneration = 0;
let videoLimitRevision = 0;
/** Senders that belonged to the last completed negotiation. */
/** Video kinds announced for the offer that is still unanswered. */
type PublishIdentity = { kind: PublishKind; trackId: string };
let openPublish: PublishIdentity[] = [];
let offeredPublish: PublishIdentity[] = [];
const publisherTracks = new Map<RtpSender, PublishIdentity>();
/** Native addTrack does not reuse an m-line that has already sent media. */
const videoSenders = new Map<PublishKind, RtpSender>();
/** Cleanup of a rejected publish must not enqueue a replacement offer. */
let discardingPublish = false;
/** Gateway reconnect left the media peer up; re-announce after our join echo. */
let republishOnJoin = false;
/** Kinds this media attempt has already announced on the gateway. */
const announced = new Set<TrackKind>();
const pendingMicRaw = new Set<MediaStream>();
const pendingCameraStreams = new Set<MediaStream>();
const pendingDisplayStreams = new Map<MediaStream, "s" | "l">();

/** Watch media peer. `watchLive` / `stopWatching` hold this object. */
let watchCall = new MediaPeer();

const ICE_DISCONNECTED_RESTART_DELAY_MS = 2_000;
/** One ICE restart, then a new ticket if it does not recover. */
const ICE_RECOVERY_DEADLINE_MS = 10_000;

type TransportRecovery = {
  generation: number;
  /** `createOffer({iceRestart:true})` has not been applied yet. */
  pendingIceRestart: boolean;
  /** A restart offer is the current local description. */
  offerCreated: boolean;
  /** ICE left `failed` after that offer, so a later failure is a new one. */
  leftFailed: boolean;
  deadlineAt: number;
};

type RecoverySlot = {
  recovery: TransportRecovery | null;
  timer: ReturnType<typeof setTimeout> | null;
};

const seatRecoverySlot: RecoverySlot = { recovery: null, timer: null };
const watchRecoverySlot: RecoverySlot = { recovery: null, timer: null };
/**
 * Bumped when that peer is stopped. In-flight offers capture it so a
 * finished attempt cannot negotiate the next one (generation numbers
 * are reused). Seat and watch stay independent.
 */
let seatEpoch = 0;
let watchEpoch = 0;
let seatReconnectTimer: ReturnType<typeof setTimeout> | null = null;
let watchReconnectTimer: ReturnType<typeof setTimeout> | null = null;
const seatRetry = new MediaRetry();
const watchRetry = new MediaRetry();

function clearRecovery(slot: RecoverySlot): void {
  slot.recovery = null;
  if (!slot.timer) return;
  clearTimeout(slot.timer);
  slot.timer = null;
}

function beginRecovery(
  slot: RecoverySlot,
  generation: number,
  stillCurrent: () => boolean,
  recovered: () => boolean,
  reconnect: () => void,
): void {
  clearRecovery(slot);
  slot.recovery = {
    generation,
    pendingIceRestart: true,
    offerCreated: false,
    leftFailed: false,
    deadlineAt: Date.now() + ICE_RECOVERY_DEADLINE_MS,
  };
  slot.timer = setTimeout(() => {
    slot.timer = null;
    if (!stillCurrent()) return;
    if (slot.recovery?.generation !== generation) return;
    if (recovered()) {
      clearRecovery(slot);
      return;
    }
    reconnect();
  }, ICE_RECOVERY_DEADLINE_MS);
}

function owesIceRestart(slot: RecoverySlot, generation: number): boolean {
  const recovery = slot.recovery;
  return Boolean(
    recovery &&
    recovery.generation === generation &&
    recovery.pendingIceRestart,
  );
}

function wantsIceRestart(
  slot: RecoverySlot,
  generation: number,
  explicit: boolean | undefined,
): boolean {
  if (explicit) return true;
  return owesIceRestart(slot, generation);
}

function markIceRestartPending(slot: RecoverySlot, generation: number): void {
  const recovery = slot.recovery;
  if (recovery?.generation === generation) recovery.pendingIceRestart = true;
}

function noteRestartOfferCreated(slot: RecoverySlot, generation: number): void {
  const recovery = slot.recovery;
  if (recovery?.generation !== generation) return;
  recovery.pendingIceRestart = false;
  recovery.offerCreated = true;
}

/** A rolled-back restart offer still has to be created. */
function noteRestartRolledBack(slot: RecoverySlot, generation: number): void {
  const recovery = slot.recovery;
  if (!recovery || recovery.generation !== generation) return;
  if (!recovery.offerCreated && !recovery.pendingIceRestart) return;
  recovery.pendingIceRestart = true;
  recovery.offerCreated = false;
}

function noteTransportProgress(
  slot: RecoverySlot,
  generation: number,
  ice: string,
  conn: string,
): void {
  const recovery = slot.recovery;
  if (
    !recovery ||
    recovery.generation !== generation ||
    !recovery.offerCreated
  ) {
    return;
  }
  if (
    ice === "checking" ||
    ice === "connected" ||
    ice === "completed" ||
    conn === "connecting"
  ) {
    recovery.leftFailed = true;
  }
}

type RecoveryAction = "start" | "retry-offer" | "reconnect" | "ignore";

function recoveryAction(
  slot: RecoverySlot,
  generation: number,
  allowReconnect: boolean,
): RecoveryAction {
  const recovery = slot.recovery;
  if (!recovery || recovery.generation !== generation) return "start";
  if (!recovery.offerCreated) return "retry-offer";
  const expired = Date.now() >= recovery.deadlineAt;
  if ((expired || recovery.leftFailed) && allowReconnect) return "reconnect";
  return "ignore";
}

function transportRecovered(pc: PeerConnection | null): boolean {
  if (!pc) return false;
  const ice = pc.iceConnectionState ?? "";
  if (ice === "connected" || ice === "completed") return true;
  if (
    ice === "failed" ||
    ice === "disconnected" ||
    ice === "checking" ||
    ice === "new" ||
    ice === "closed"
  ) {
    return false;
  }
  return (pc.connectionState ?? "") === "connected";
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
    channel: state.channelId ?? state.watchChannelId ?? undefined,
    server: state.serverId ?? state.watchServerId ?? undefined,
    ...extra,
  };
  if (level === "warn") console.warn("[gelabber:voice]", fields);
  else console.info("[gelabber:voice]", fields);
}

function currentUserId(): string | null {
  return deps?.userId && deps.userId !== currentUserId
    ? deps.userId()
    : (useSession.getState().user?.id ?? null);
}

function defaultCreatePeer(iceServers: IceServer[]): PeerConnection {
  return new RTCPeerConnection({ iceServers }) as unknown as PeerConnection;
}

async function defaultGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia(constraints);
}

async function defaultGetDisplayMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  return navigator.mediaDevices.getDisplayMedia(constraints);
}

function defaultAttachRemote(stream: MediaStream): void {
  if (typeof Audio === "undefined") return;
  if (!remoteAudio) {
    remoteAudio = new Audio();
    remoteAudio.autoplay = true;
    remoteAudio.setAttribute("playsinline", "true");
  }
  remoteAudio.srcObject = stream;
  applyLocalAudio();
  applyPlayback();
  playAudio(remoteAudio);
}

function updatePlaybackBlocked(): void {
  useVoice.setState({ playbackBlocked: blockedPlayback.size > 0 });
}

function playAudio(el: HTMLAudioElement): void {
  const stream = el.srcObject;
  if (!stream) return;
  const failed = (error: unknown) => {
    if (
      el.srcObject !== stream ||
      (error as { name?: string })?.name === "AbortError"
    )
      return;
    blockedPlayback.add(el);
    updatePlaybackBlocked();
  };
  try {
    void el
      .play()
      ?.then(() => {
        if (el.srcObject !== stream) return;
        blockedPlayback.delete(el);
        updatePlaybackBlocked();
      })
      .catch(failed);
  } catch (error) {
    failed(error);
  }
}

/** Call synchronously from a click so the browser grants playback activation. */
export function retryPlayback(): void {
  for (const el of [remoteAudio, ...watchAudio.values()]) {
    if (el?.srcObject) playAudio(el);
  }
}

function rtpSenderCtor():
  | {
      getCapabilities?: (
        kind: string,
      ) => { codecs: { mimeType: string }[] } | null;
    }
  | undefined {
  return (
    globalThis as unknown as {
      RTCRtpSender?: {
        getCapabilities?: (
          kind: string,
        ) => { codecs: { mimeType: string }[] } | null;
      };
    }
  ).RTCRtpSender;
}

function preferOpus(pc: PeerConnection): void {
  const caps = rtpSenderCtor()?.getCapabilities?.("audio");
  if (!caps) return;
  const preferred = [
    ...caps.codecs.filter((c) => c.mimeType.toLowerCase() === "audio/opus"),
    ...caps.codecs.filter((c) => c.mimeType.toLowerCase() !== "audio/opus"),
  ];
  for (const transceiver of pc.getTransceivers?.() ?? []) {
    const kind =
      transceiver.sender?.track?.kind ?? transceiver.receiver?.track?.kind;
    if (kind === "audio") {
      transceiver.setCodecPreferences?.(preferred);
    }
  }
}

function preferVp8(pc: PeerConnection): void {
  const caps = rtpSenderCtor()?.getCapabilities?.("video");
  if (!caps) return;
  const preferred = [
    ...caps.codecs.filter((c) => c.mimeType.toLowerCase() === "video/vp8"),
    ...caps.codecs.filter((c) => c.mimeType.toLowerCase() !== "video/vp8"),
  ];
  for (const transceiver of pc.getTransceivers?.() ?? []) {
    const kind =
      transceiver.sender?.track?.kind ?? transceiver.receiver?.track?.kind;
    if (kind === "video") {
      transceiver.setCodecPreferences?.(preferred);
    }
  }
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

function withTunedSdp(
  desc: { type: string; sdp?: string },
  pc = seat.pc,
): {
  type: string;
  sdp?: string;
} {
  if (!desc.sdp) return desc;
  return { type: desc.type, sdp: tuneSeatAudioSdp(desc.sdp, pc) };
}

function tuneSeatAudioSdp(sdp: string, pc = seat.pc): string {
  const tracks = new Set<string>();
  const mids = new Set<string>();
  for (const [sender, identity] of pc === seat.pc ? publisherTracks : []) {
    if (identity.kind !== "sa" && identity.kind !== "la") continue;
    tracks.add(identity.trackId);
    if (sender.track) tracks.add(sender.track.id);
    const mid = pc
      ?.getTransceivers?.()
      .find((item) => item.sender === sender)?.mid;
    if (mid) mids.add(mid);
  }
  for (const [mid, track] of publishedTrackIds(
    pc?.remoteDescription?.sdp ?? "",
  )) {
    if (/:(?:sa|la)(?:[-:]|$)/.test(track)) mids.add(mid);
  }
  return tuneAudioSdp(sdp, audioBitrate(), tracks, mids);
}

export function configureVoice(next: Partial<VoiceDeps>): void {
  const gateway = next.gateway ?? deps?.gateway ?? getGateway();
  deps = {
    gateway,
    userId: next.userId ?? deps?.userId ?? currentUserId,
    createPeer: next.createPeer ?? deps?.createPeer ?? defaultCreatePeer,
    getUserMedia:
      next.getUserMedia ?? deps?.getUserMedia ?? defaultGetUserMedia,
    getDisplayMedia:
      next.getDisplayMedia ?? deps?.getDisplayMedia ?? defaultGetDisplayMedia,
    fetchTicket: next.fetchTicket ?? deps?.fetchTicket ?? requestMediaTicket,
    openMedia: next.openMedia ?? deps?.openMedia ?? openMediaSocket,
    attachRemote:
      next.attachRemote ?? deps?.attachRemote ?? defaultAttachRemote,
    onError: next.onError ?? deps?.onError ?? notifyError,
  };
  ensureBound();
}

function ensureBound(): void {
  if (bound) return;
  const gateway = deps?.gateway ?? getGateway();
  if (!deps) {
    deps = {
      gateway,
      userId: currentUserId,
      createPeer: defaultCreatePeer,
      getUserMedia: defaultGetUserMedia,
      getDisplayMedia: defaultGetDisplayMedia,
      fetchTicket: requestMediaTicket,
      openMedia: openMediaSocket,
      attachRemote: defaultAttachRemote,
      onError: notifyError,
    };
  }
  gateway.onSig(onSig);
  gateway.onErr(onErr);
  gateway.onReady(onReady);
  onMediaSettingsChange(handleSettingsChange);
  bound = true;
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

/** Video still arriving on the open media peer, keyed by publisher. */
const receivedVideo = new Map<
  string,
  Partial<Record<"v" | "s" | "l", MediaStream>>
>();

function noteReceived(
  userId: string,
  kind: "v" | "s" | "l",
  stream: MediaStream,
  track: MediaStreamTrack,
): void {
  // Receiver tracks can keep their identity when the SFU reuses a transceiver.
  // Retire the old tile alias before assigning that receiver to its current MSID.
  for (const [publisher, sources] of receivedVideo) {
    for (const previousKind of ["v", "s", "l"] as const) {
      if (
        (publisher !== userId || previousKind !== kind) &&
        sources[previousKind]?.getVideoTracks().includes(track)
      ) {
        forgetReceived(publisher, previousKind);
      }
    }
  }
  const prev = receivedVideo.get(userId) ?? {};
  receivedVideo.set(userId, { ...prev, [kind]: stream });
  track.addEventListener("ended", () => {
    const held = receivedVideo.get(userId);
    if (!held || held[kind] !== stream) return;
    const next = { ...held };
    delete next[kind];
    if (!next.v && !next.s && !next.l) receivedVideo.delete(userId);
    else receivedVideo.set(userId, next);
    dropRemote(userId, kind);
  });
}

/** An explicit unpublish ends that tile. A later `t:"p"` must not restore it. */
function forgetReceived(userId: string, kind: "v" | "s" | "l"): void {
  const held = receivedVideo.get(userId);
  if (held?.[kind]) {
    const next = { ...held };
    delete next[kind];
    if (!next.v && !next.s && !next.l) receivedVideo.delete(userId);
    else receivedVideo.set(userId, next);
  }
  dropRemote(userId, kind);
}

function reattachReceived(userId: string, kind: "v" | "s" | "l"): void {
  const stream = receivedVideo.get(userId)?.[kind];
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
  receivedVideo.clear();
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

/** SFU stream id is `{userId}:{v|s|l}`; track id may be `{userId}:{k}-{ssrc}`. */
export function parseRemoteStreamId(
  id: string,
): { userId: string; k: PublishKind } | null {
  const match = /^(.*):(sa|la|v|s|l)(?:[-:].*)?$/.exec(id);
  const userId = match?.[1];
  const k = match?.[2];
  if (
    userId &&
    (k === "v" || k === "s" || k === "l" || k === "sa" || k === "la")
  ) {
    return { userId, k };
  }
  return null;
}

let soundOnJoin = false;

function onSig(event: SigEvent & { lc?: string }): void {
  const state = useVoice.getState();
  const watchingHere =
    state.watching &&
    event.s === state.watchServerId &&
    event.c === state.watchChannelId;
  const inRoom =
    state.status === "joined" &&
    event.s === state.serverId &&
    event.c === state.channelId;
  if (watchingHere && event.u === state.watchPublisherId) {
    if (event.t === "u" && event.k === "la")
      clearSourcePlayback("watch", event.u, "l");
    if (event.t === "p" && event.k === "l") clearWatchPublisherTimer();
    else if (event.t === "l" && watchPublisherTimer === null) {
      // Gateway detach can be followed by a fresh seat/claim while the native
      // media peer survives. SFU authority still stops revoked publications;
      // keep the user's Watch intent for a bounded replacement handshake.
      watchPublisherTimer = setTimeout(() => {
        watchPublisherTimer = null;
        stopWatching();
      }, 20000);
    }
  }
  if (
    watchingHere &&
    event.t === "u" &&
    event.k === "l" &&
    (!state.watchPublisherId || event.u === state.watchPublisherId)
  ) {
    stopWatching();
  }
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
        awaitingJoin &&
        event.c === awaitingJoin.channelId
      ) {
        awaitingJoin = null;
        if (soundOnJoin) {
          soundOnJoin = false;
          playCallSound("join");
        }
        if (republishOnJoin) {
          republishOnJoin = false;
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
      if (event.t === "p" && event.k === "l" && userId === currentUserId()) {
        if (state.live && isLiveNonce(event.lc)) {
          if (event.lc === withdrawnLiveNonce) {
            stopLocalVideo("l");
            deps?.onError?.(
              new ApiError("forbidden", 0, errorMessage("forbidden")),
            );
            return;
          }
          const changed = liveClaimNonce !== event.lc;
          if (liveClaimTimer !== null) clearTimeout(liveClaimTimer);
          liveClaimTimer = null;
          liveClaimNonce = event.lc;
          withdrawnLiveNonce = null;
          liveRecoveryDeadline = null;
          awaitingLive = null;
          if (changed && liveStream) void publishLocal("l", liveStream);
        }
      }
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
      if (event.t === "u" && (event.k === "sa" || event.k === "la")) {
        const kind = event.k === "sa" ? "s" : "l";
        clearSourcePlayback("voice", userId, kind);
        for (const [track, source] of receivedSourceAudio)
          if (source.userId === userId && source.kind === kind)
            receivedSourceAudio.delete(track);
      }
      if (event.k === "v" || event.k === "s" || event.k === "l") {
        if (event.t === "u") {
          // Gateway `t:"l"` keeps a track that is still arriving. `t:"u"`
          // means that camera, screen, or Go Live actually ended.
          forgetReceived(userId, event.k);
          if (event.k !== "v") {
            clearSourcePlayback("voice", userId, event.k);
            for (const [track, source] of receivedSourceAudio) {
              if (source.userId === userId && source.kind === event.k)
                receivedSourceAudio.delete(track);
            }
          }
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
  const pending = awaitingJoin;
  if (
    pending &&
    err.s === pending.serverId &&
    err.c === pending.channelId &&
    (err.e === "forbidden" || err.e === "not_found" || err.e === "bad_request")
  ) {
    const state = useVoice.getState();
    if (state.channelId === pending.channelId) {
      awaitingJoin = null;
      awaitingLive = null;
      stopPeer();
      useVoice.setState({
        ...idle,
        watching: state.watching,
        watchServerId: state.watchServerId,
        watchChannelId: state.watchChannelId,
        watchChannelName: state.watchChannelName,
        watchPublisherId: state.watchPublisherId,
        watchStream: state.watchStream,
        playbackBlocked: state.playbackBlocked,
      });
      const code: ApiErrorCode =
        err.e === "forbidden" ||
        err.e === "not_found" ||
        err.e === "bad_request"
          ? err.e
          : "bad_request";
      deps?.onError?.(new ApiError(code, 0, errorMessage(code)));
    }
    return;
  }
  if (
    awaitingLive &&
    !awaitingJoin &&
    err.s === awaitingLive.serverId &&
    err.c === awaitingLive.channelId &&
    (err.e === "forbidden" || err.e === "bad_request")
  ) {
    awaitingLive = null;
    if (useVoice.getState().live) {
      stopLocalVideo("l");
    }
    const code: ApiErrorCode =
      err.e === "forbidden" || err.e === "bad_request" ? err.e : "bad_request";
    deps?.onError?.(new ApiError(code, 0, errorMessage(code)));
  }
}

function onReady(): void {
  soundOnJoin = false;
  stopCallSounds();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  const self = currentUserId();
  const keepMedia = seat.isOpen();
  useVoice.setState({
    participants: self
      ? { [self]: state.participants[self] ?? { pubs: [] } }
      : {},
    // A live peer connection does not fire ontrack again. Keep the
    // streams VoiceRoom is already showing.
    remote: keepMedia ? state.remote : {},
  });
  occupySelf(state);
  awaitingJoin = { serverId: state.serverId, channelId: state.channelId };
  republishOnJoin = keepMedia;
  deps?.gateway.send({
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

function clearSeatReconnectTimer(): void {
  if (!seatReconnectTimer) return;
  clearTimeout(seatReconnectTimer);
  seatReconnectTimer = null;
}

function clearWatchReconnectTimer(): void {
  if (!watchReconnectTimer) return;
  clearTimeout(watchReconnectTimer);
  watchReconnectTimer = null;
}

function streamDetail(kind: "v" | "s" | "l"): "camera" | "screen" | "live" {
  if (kind === "v") return "camera";
  if (kind === "s") return "screen";
  return "live";
}

function noteStream(kind: "v" | "s" | "l", started: boolean): void {
  const state = useVoice.getState();
  noteDiagnosticEvent({
    kind: started ? "stream-start" : "stream-stop",
    connection: "voice",
    detail: streamDetail(kind),
    streaming: state.camera || state.sharing || state.live,
  });
}

function diagnosticVideoSources(): Record<string, VideoSource> {
  const sources: Record<string, VideoSource> = {};
  const camera = cameraStream?.getVideoTracks()[0];
  const screen = screenStream?.getVideoTracks()[0];
  const live = liveStream?.getVideoTracks()[0];
  if (camera) sources[camera.id] = "camera";
  if (screen) sources[screen.id] = "screen";
  if (live) sources[live.id] = "live";
  return sources;
}

function diagnosticAudioSources(): Record<string, AudioSource> {
  const sources: Record<string, AudioSource> = {};
  for (const track of screenStream?.getAudioTracks() ?? [])
    sources[track.id] = "screen-audio";
  for (const track of liveStream?.getAudioTracks() ?? [])
    sources[track.id] = "live-audio";
  for (const [track, source] of receivedAudioSources)
    sources[track.id] = source.source;
  for (const [track, source] of sourceAudio)
    sources[track.id] = source.kind === "s" ? "screen-audio" : "live-audio";
  return sources;
}

function voiceStreaming(): boolean {
  const state = useVoice.getState();
  return state.camera || state.sharing || state.live;
}

function voiceCaps(): Caps {
  const base = defaultCaps();
  const senders = (seat.pc?.getSenders?.() ?? []).filter(
    (sender) => sender.track?.kind === "video",
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
  for (const sender of seat.pc?.getSenders?.() ?? []) {
    if (sender.track?.kind !== "audio") continue;
    const encodings = sender.getParameters?.().encodings;
    if (!encodings?.length) continue;
    const kind = publisherTracks.get(sender)?.kind;
    const source: AudioSource =
      kind === "sa" ? "screen-audio" : kind === "la" ? "live-audio" : "voice";
    audioLimits[source] = encodings.every(
      (encoding) => typeof encoding.maxBitrate === "number",
    )
      ? encodings.reduce((sum, encoding) => sum + encoding.maxBitrate!, 0)
      : null;
  }
  const active = Object.values(videoLimits);
  return {
    ...base,
    audioLimits,
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
      const pc = seat.pc;
      if (seat.generation !== generation || !pc?.getStats) return null;
      try {
        return statsEntriesFromReport(await pc.getStats());
      } catch {
        return null;
      }
    },
  });
}

function attachWatchDiagnostics(generation: number): void {
  attachDiagnostics({
    role: "watch",
    caps: defaultCaps,
    audioSources: diagnosticAudioSources,
    streaming: () => false,
    getReport: async () => {
      const pc = watchCall.pc;
      if (watchCall.generation !== generation || !pc?.getStats) return null;
      try {
        return statsEntriesFromReport(await pc.getStats());
      } catch {
        return null;
      }
    },
  });
}

function noteIceState(
  pc: PeerConnection,
  role: "voice" | "watch",
  generation: number,
  current: () => number,
): void {
  if (current() !== generation) return;
  const state = pc.iceConnectionState;
  if (state === "failed" || state === "disconnected") {
    noteDiagnosticEvent({
      kind: "ice-error",
      connection: role,
      detail: state,
    });
  } else if (state === "connected" || state === "completed") {
    noteDiagnosticEvent({
      kind: "recovery",
      connection: role,
      detail: state,
    });
  }
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

function stopPeer(preserveCapture = false): void {
  if (!preserveCapture) micForceBrowser = false;
  if (!preserveCapture) clearLiveClaim();
  seatRetry.cancel(!preserveCapture);
  if (preserveCapture) {
    for (const [kind, stream] of [
      ["v", cameraStream],
      ["s", screenStream],
      ["l", liveStream],
    ] as const) {
      if (stream && !hasLiveTrack(stream, "video")) stopLocalVideo(kind);
    }
  }
  streamReported = false;
  seatEpoch += 1;
  clearSeatReconnectTimer();
  clearRecovery(seatRecoverySlot);
  detachDiagnostics("voice");
  seat.close();
  seatMediaVersion = null;
  micEpoch += 1;
  cameraEpoch += 1;
  cameraProfileEpoch += 1;
  screenProfileEpoch += 1;
  screenEpoch += 1;
  liveEpoch += 1;
  audioCommitChain = Promise.resolve();
  cameraCommitChain = Promise.resolve();
  resetVideoLimitQueue();
  openPublish = [];
  offeredPublish = [];
  publisherTracks.clear();
  videoSenders.clear();
  discardingPublish = false;
  announced.clear();
  for (const stream of pendingMicRaw) stopTracks(stream);
  pendingMicRaw.clear();
  for (const stream of pendingCameraStreams) stopTracks(stream);
  pendingCameraStreams.clear();
  for (const stream of pendingDisplayStreams.keys()) stopTracks(stream);
  pendingDisplayStreams.clear();
  if (
    !preserveCapture ||
    !hasLiveTrack(localStream, "audio") ||
    (rawMicStream && !hasLiveTrack(rawMicStream, "audio"))
  ) {
    stopTracks(localStream);
    stopTracks(rawMicStream);
    disposeMicGain();
    localStream = null;
    rawMicStream = null;
  }
  if (!preserveCapture || !hasLiveTrack(cameraStream, "video")) {
    stopTracks(cameraStream);
    cameraStream = null;
  }
  if (!preserveCapture || !hasLiveTrack(screenStream, "video")) {
    stopTracks(screenStream);
    screenStream = null;
  }
  if (!preserveCapture || !hasLiveTrack(liveStream, "video")) {
    stopTracks(liveStream);
    liveStream = null;
  }
  remoteMix = null;
  clearSourcePlayback("voice");
  receivedSourceAudio.clear();
  for (const [track, source] of receivedAudioSources)
    if (source.role === "voice") receivedAudioSources.delete(track);
  clearReceived();
  if (remoteAudio) {
    remoteAudio.srcObject = null;
    blockedPlayback.delete(remoteAudio);
    updatePlaybackBlocked();
  }
  useVoice.setState({
    localCamera: cameraStream,
    localScreen: screenStream,
    localLive: liveStream,
    sourceAudio: preserveCapture
      ? useVoice.getState().sourceAudio
      : { s: "off", l: "off" },
    ...(preserveCapture
      ? {
          camera: !!cameraStream,
          sharing: !!screenStream,
          live: !!liveStream,
        }
      : {}),
    remote: {},
  });
}

function rollbackSeat(error?: unknown): void {
  const state = useVoice.getState();
  const serverId = state.serverId;
  const channelId = state.channelId;
  awaitingJoin = null;
  awaitingLive = null;
  republishOnJoin = false;
  if (state.live && serverId && channelId) {
    applyLiveEnd(serverId, channelId, currentUserId() ?? undefined);
  }
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...idle,
    watching: state.watching,
    watchServerId: state.watchServerId,
    watchChannelId: state.watchChannelId,
    watchChannelName: state.watchChannelName,
    watchPublisherId: state.watchPublisherId,
    watchStream: state.watchStream,
    playbackBlocked: blockedPlayback.size > 0,
  });
  applyPlayback();
  if (serverId && channelId) {
    deps?.gateway.send({
      op: "sig",
      t: "l",
      s: serverId,
      c: channelId,
    });
  }
  if (error !== undefined) {
    deps?.onError?.(error);
  }
}

function applyLocalAudio(): void {
  const state = useVoice.getState();
  const micOff = state.muted || state.deafened;
  localStream?.getAudioTracks().forEach((track) => {
    track.enabled = !micOff;
  });
  rawMicStream?.getAudioTracks().forEach((track) => {
    track.enabled = !micOff;
  });
  applyPlayback();
}

function playbackVolume(): number {
  if (useVoice.getState().deafened) return 0;
  return useMediaSettings.getState().outputVolume;
}

function applyPlayback(): void {
  const volume = playbackVolume();
  const deafened = useVoice.getState().deafened;
  for (const el of [remoteAudio, ...watchAudio.values()]) {
    if (!el) continue;
    el.muted = deafened;
    el.volume = volume;
    void applySink(el);
  }
  const settings = useMediaSettings.getState();
  for (const { el } of sourceAudio.values()) {
    el.muted = deafened || settings.sourceAudioMuted;
    el.volume = deafened ? 0 : settings.sourceAudioVolume;
    void applySink(el);
  }
}

async function applySink(el: HTMLAudioElement): Promise<void> {
  const id = useMediaSettings.getState().audioOutputId;
  const sink = el as HTMLAudioElement & {
    setSinkId?: (deviceId: string) => Promise<void>;
  };
  if (!sink.setSinkId) return;
  try {
    await sink.setSinkId(id);
  } catch {
    // unplugged / permission
  }
}

async function applySendBitrate(): Promise<void> {
  const bitrate = audioBitrate();
  for (const sender of seat.pc?.getSenders?.() ?? []) {
    if (sender.track?.kind !== "audio") continue;
    const params = sender.getParameters?.();
    if (!params?.encodings.length) continue;
    for (const encoding of params.encodings) {
      const cap =
        publisherTracks.get(sender)?.kind === "sa" ||
        publisherTracks.get(sender)?.kind === "la"
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
  }
}

function profileForVideoTrack(track: MediaStreamTrack): StreamProfileId {
  const camera = cameraStream?.getVideoTracks().some((item) => item === track);
  if (camera) return useMediaSettings.getState().cameraProfile;
  return useMediaSettings.getState().screenProfile;
}

function resetVideoLimitQueue(): void {
  videoLimitRevision += 1;
  videoLimitChain = Promise.resolve();
  videoLimitGeneration = seat.generation;
}

function videoLimitCurrent(
  pc: PeerConnection,
  generation: number,
  revision: number,
): boolean {
  return (
    videoLimitRevision === revision &&
    videoLimitGeneration === generation &&
    seat.generation === generation &&
    seat.pc === pc
  );
}

/** Serialize budget writes. Profiles are read when the pass runs, not when queued. */
function enqueueVideoLimits(pc: PeerConnection): Promise<void> {
  const generation = seat.generation;
  if (videoLimitGeneration !== generation) {
    videoLimitChain = Promise.resolve();
    videoLimitGeneration = generation;
  }
  const revision = ++videoLimitRevision;
  const run = videoLimitChain.then(
    () => applyVideoLimits(pc, generation, revision),
    () => applyVideoLimits(pc, generation, revision),
  );
  videoLimitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

async function applyVideoLimits(
  pc: PeerConnection,
  generation: number,
  revision: number,
): Promise<void> {
  if (!videoLimitCurrent(pc, generation, revision)) return;
  const senders = (pc.getSenders?.() ?? []).filter(
    (s) => s.track?.kind === "video",
  );
  const profiles = senders.map((sender) =>
    sender.track ? profileForVideoTrack(sender.track) : "balanced",
  );
  const shares = allocateVideoBitrates(
    profiles,
    useMediaSettings.getState().videoUploadLimit,
    useMediaSettings.getState().economyMode,
  );
  for (let index = 0; index < senders.length; index += 1) {
    if (!videoLimitCurrent(pc, generation, revision)) return;
    const sender = senders[index];
    const params = sender?.getParameters?.();
    if (!sender || !params?.encodings.length) continue;
    const share = shares[index] ?? null;
    const perEncoding =
      share === null ? null : Math.floor(share / params.encodings.length);
    const fps = streamProfileFps(profiles[index] ?? "balanced");
    for (const encoding of params.encodings) {
      if (perEncoding === null) delete encoding.maxBitrate;
      else encoding.maxBitrate = perEncoding;
      encoding.maxFramerate = fps;
    }
    try {
      await sender.setParameters?.(params);
    } catch {
      // Some browsers require negotiation first; retry after SDP completes.
    }
    // This snapshot is stale. Stop before the next sender; the newer pass
    // applies the latest budgets across every sender still sending.
    if (!videoLimitCurrent(pc, generation, revision)) return;
  }
}

function liveVideoTracks(kind: StreamKind): MediaStreamTrack[] {
  const streams =
    kind === "camera" ? [cameraStream] : [screenStream, liveStream];
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
  const epoch = kind === "camera" ? ++cameraProfileEpoch : ++screenProfileEpoch;
  const current = () =>
    (kind === "camera" ? cameraProfileEpoch : screenProfileEpoch) === epoch;
  const pc = seat.pc;
  if (pc) {
    try {
      await enqueueVideoLimits(pc);
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

async function captureVideo(kind: "v" | "s" | "l"): Promise<MediaStream> {
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
      ? (deps?.getUserMedia ?? defaultGetUserMedia)
      : (deps?.getDisplayMedia ?? defaultGetDisplayMedia);
  if (kind !== "v") {
    // One browser picker supplies both tracks; profile fallback never reopens it.
    const stream = await getMedia({
      audio: settings.shareSourceAudio
        ? { ...SOURCE_AUDIO_CONSTRAINTS }
        : false,
      video: ladder[0] ?? true,
    });
    pendingDisplayStreams.set(stream, kind);
    if (!settings.shareSourceAudio)
      stream.getAudioTracks().forEach((track) => track.stop());
    const track = stream.getVideoTracks()[0];
    if (track?.applyConstraints) {
      for (let index = 0; index < ladder.length; index += 1) {
        try {
          await track.applyConstraints(ladder[index]);
          if (index > 0) deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
          break;
        } catch (error) {
          if (track.readyState === "ended") break;
          if (!isOverconstrainedError(error) || index + 1 === ladder.length) {
            // Capture already succeeded. Keep the browser's usable source.
            deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
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
      if (index > 0) deps?.onError?.(new Error(STREAM_PROFILE_FALLBACK));
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
  const insert = activeMicGain;
  activeMicGain = null;
  insert?.dispose();
  noteAudioProcessing(null);
}

function enqueueAudioCommit(job: () => Promise<void>): Promise<void> {
  const run = audioCommitChain.then(job, job);
  audioCommitChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function enqueueCameraCommit(job: () => Promise<void>): Promise<void> {
  const run = cameraCommitChain.then(job, job);
  cameraCommitChain = run.then(
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
  pc: PeerConnection,
  session: number,
  request: number,
): boolean {
  return (
    seat.pc === pc &&
    seat.generation === session &&
    micEpoch === request &&
    useVoice.getState().status === "joined"
  );
}

function cameraCurrent(
  pc: PeerConnection,
  session: number,
  epoch: number,
): boolean {
  return (
    seat.pc === pc &&
    seat.generation === session &&
    cameraEpoch === epoch &&
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
  pc: PeerConnection,
  send: MediaStream,
  session: number,
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
  const sender = audioSender();
  if (sender?.replaceTrack) {
    await sender.replaceTrack(track);
    return "replaced";
  }
  if (seat.pc === pc) {
    pc.addTrack?.(track, send);
    seat.needOffer = true;
    void offerIfStable(session);
    return "added";
  }
  return "none";
}

function failedMicProcessor(owner: MicProcessor): void {
  if (activeMicGain !== owner) return;
  const sender = audioSender(),
    generation = seat.generation;
  micForceBrowser = true;
  disableAudio(owner.stream);
  rawMicStream?.getTracks().forEach((track) => track.stop());
  owner.dispose();
  activeMicGain = null;
  localStream = null;
  rawMicStream = null;
  noteAudioProcessing({
    ...owner.info,
    actual: null,
    message: "Audioprozessor ausgefallen; Ersatzmikrofon wird aktiviert",
    contextState: "closed",
  });
  deps?.onError?.(
    new Error(
      "Audioprozessor ausgefallen; lokale Aufnahme beendet, Browser-Ersatz wird aktiviert",
    ),
  );
  void enqueueAudioCommit(async () => {
    await replaceSenderTrack(sender, null).catch(() => undefined);
  }).then(() => {
    if (
      seat.generation === generation &&
      useVoice.getState().status === "joined"
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
    micForceBrowser,
    () => {
      if (owned.processor) failedMicProcessor(owned.processor);
    },
    {
      current,
      acquired(raw) {
        disableAudio(raw);
        pendingMicRaw.add(raw);
      },
      discarded(raw) {
        pendingMicRaw.delete(raw);
      },
    },
    (info) => {
      if (owned.processor && activeMicGain === owned.processor)
        noteAudioProcessing(info);
    },
  );
  owned.processor = result.processor;
  const latest = useMediaSettings.getState();
  const gain =
    micForceBrowser || result.processor.nativeFallback ? 1 : latest.inputGain;
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
          if (owned.processor && activeMicGain === owned.processor)
            noteAudioProcessing(info);
        },
      );
      previous.dispose();
    } catch (error) {
      result.processor.dispose();
      stopTracks(result.raw);
      pendingMicRaw.delete(result.raw);
      if (current() && !micForceBrowser) {
        micForceBrowser = true;
        return acquireMic(getMedia, current);
      }
      throw error;
    }
  } else result.processor.setGain(gain);
  owned.processor = result.processor;
  if (!current() || !result.processor.usable()) {
    result.processor.dispose();
    stopTracks(result.raw);
    pendingMicRaw.delete(result.raw);
    if (current() && !micForceBrowser) {
      micForceBrowser = true;
      return acquireMic(getMedia, current);
    }
    throw new Error(
      "Mikrofonanfrage abgebrochen oder Audioprozessor ausgefallen",
    );
  }
  return result;
}

function discardFailedMicCandidate(
  raw: MediaStream,
  insert: MicProcessor,
  current: boolean,
): void {
  insert.dispose();
  stopTracks(raw);
  pendingMicRaw.delete(raw);
  if (current && !micForceBrowser) {
    micForceBrowser = true;
    deps?.onError?.(
      new Error(
        "Audioprozessor vor Mikrofonwechsel ausgefallen; Browser-Ersatz wird aktiviert",
      ),
    );
    void refreshMic();
  }
}

function announceMicrophone(): void {
  if (announced.has("a")) return;
  const { serverId, channelId } = useVoice.getState();
  if (!serverId || !channelId) return;
  const self = currentUserId();
  if (self) setPub(self, "a", true);
  announced.add("a");
  deps?.gateway.send({ op: "sig", t: "p", s: serverId, c: channelId, k: "a" });
}

async function refreshMic(): Promise<void> {
  const pc = seat.pc;
  if (!pc || useVoice.getState().status !== "joined") return;
  const session = seat.generation;
  const request = ++micEpoch;
  const getUserMedia = deps?.getUserMedia ?? defaultGetUserMedia;
  let candidate: Awaited<ReturnType<typeof acquireMic>>;
  try {
    candidate = await acquireMic(getUserMedia, () =>
      micCurrent(pc, session, request),
    );
  } catch (error) {
    if (micCurrent(pc, session, request)) deps?.onError?.(error);
    return;
  }
  const { raw, processor: insert } = candidate;
  const send = insert.stream;
  if (!micCurrent(pc, session, request)) {
    insert.dispose();
    stopTracks(raw);
    return;
  }
  disableAudio(raw);
  pendingMicRaw.add(raw);
  disableAudio(send);
  await enqueueAudioCommit(async () => {
    if (!micCurrent(pc, session, request)) {
      insert?.dispose();
      stopTracks(raw);
      pendingMicRaw.delete(raw);
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
      outcome = await commitMicSend(pc, send, session);
    } catch (error) {
      insert?.dispose();
      stopTracks(raw);
      pendingMicRaw.delete(raw);
      if (micCurrent(pc, session, request)) {
        deps?.onError?.(
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
      if (seat.generation === session && seat.pc === pc) {
        try {
          if (outcome === "replaced") {
            await replaceSenderTrack(sender, previous);
          } else if (outcome === "added" && sender?.replaceTrack) {
            await replaceSenderTrack(sender, null);
          }
        } catch {
          // seat.pc may already be tearing down
        }
      }
      discardFailedMicCandidate(raw, insert, micCurrent(pc, session, request));
      return;
    }
    const oldRaw = rawMicStream;
    const oldSend = localStream;
    const oldInsert = activeMicGain;
    rawMicStream = raw;
    localStream = send;
    activeMicGain = insert;
    noteAudioProcessing(insert.info);
    pendingMicRaw.delete(raw);
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
  const pc = seat.pc;
  if (!pc || !useVoice.getState().camera) return;
  const session = seat.generation;
  const epoch = ++cameraEpoch;
  let stream: MediaStream;
  try {
    stream = await captureVideo("v");
  } catch (error) {
    if (isOverconstrainedError(error) && cameraCurrent(pc, session, epoch)) {
      deps?.onError?.(
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
  pendingCameraStreams.add(stream);
  await enqueueCameraCommit(async () => {
    if (!cameraCurrent(pc, session, epoch)) {
      stopTracks(stream);
      pendingCameraStreams.delete(stream);
      return;
    }
    bindEnded(stream, "v");
    const sender = cameraSender();
    const previous = sender?.track ?? null;
    try {
      if (sender?.replaceTrack) {
        await sender.replaceTrack(track);
      }
    } catch (error) {
      stopTracks(stream);
      pendingCameraStreams.delete(stream);
      if (cameraCurrent(pc, session, epoch)) {
        deps?.onError?.(
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
        seat.generation === session &&
        seat.pc === pc &&
        sender?.replaceTrack
      ) {
        try {
          await sender.replaceTrack(previous);
        } catch {
          // seat.pc may already be tearing down
        }
      }
      stopTracks(stream);
      pendingCameraStreams.delete(stream);
      return;
    }
    const old = cameraStream;
    cameraStream = stream;
    track.enabled = true;
    useVoice.setState({ camera: true, localCamera: stream });
    pendingCameraStreams.delete(stream);
    stopTracks(old);
    if (!sender?.replaceTrack) {
      await publishLocal("v", stream);
    }
  });
}

async function applyInputGain(): Promise<void> {
  await enqueueAudioCommit(async () => {
    if (!rawMicStream || useVoice.getState().status !== "joined" || !seat.pc)
      return;
    const settings = useMediaSettings.getState();
    if (activeMicGain && activeMicGain.stream !== rawMicStream) {
      activeMicGain.setGain(settings.inputGain);
      return;
    }
    if (settings.inputGain === 1) return;
    const pc = seat.pc,
      raw = rawMicStream,
      session = seat.generation,
      request = micEpoch;
    const actual = activeMicGain?.info?.actual ?? "browser";
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
          if (activeMicGain === insert) noteAudioProcessing(info);
        },
      );
    } catch (error) {
      deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      insert.dispose();
      if (micCurrent(pc, session, request))
        deps?.onError?.(
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
      deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request) || !insert.usable()) {
      if (seat.pc === pc && seat.generation === session)
        await replaceSenderTrack(sender, previous).catch(() => undefined);
      insert.dispose();
      if (micCurrent(pc, session, request))
        deps?.onError?.(
          new Error("Mic-Gain ausgefallen; bisheriges Mikrofon bleibt aktiv"),
        );
      return;
    }
    activeMicGain?.dispose();
    activeMicGain = insert;
    localStream = insert.stream;
    noteAudioProcessing(insert.info);
    applyLocalAudio();
  });
}

function queueInputGain(): void {
  void applyInputGain();
}

function audioSender(): RtpSender | undefined {
  return seat.pc
    ?.getSenders?.()
    .find(
      (sender) =>
        sender.track?.kind === "audio" && !publisherTracks.has(sender),
    );
}

function cameraSender(): RtpSender | undefined {
  const cam = cameraStream?.getVideoTracks()[0];
  if (!cam) return undefined;
  return seat.pc?.getSenders?.().find((sender) => sender.track === cam);
}

function handleSettingsChange(prev: MediaSettings, next: MediaSettings): void {
  applyLocalAudio();
  const joined = useVoice.getState().status === "joined";
  const recaptureMic =
    prev.audioInputId !== next.audioInputId ||
    prev.echoCancellation !== next.echoCancellation ||
    prev.noiseSuppression !== next.noiseSuppression ||
    prev.autoGainControl !== next.autoGainControl ||
    prev.processingMode !== next.processingMode;
  if (recaptureMic) micForceBrowser = false;
  const camChanged = prev.videoInputId !== next.videoInputId;
  const qualityChanged =
    prev.quality !== next.quality ||
    prev.economyMode !== next.economyMode ||
    prev.processingMode !== next.processingMode;
  if (joined || useVoice.getState().watching) {
    const connection = joined ? "voice" : "watch";
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
    seat.pc
  ) {
    void enqueueVideoLimits(seat.pc);
  }
  if (joined && qualityChanged) {
    void applySendBitrate();
    seat.needOffer = true;
    void offerIfStable(seat.generation);
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
  deps?.gateway.send({
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
  if (on) announced.add(kind);
  else announced.delete(kind);
  deps?.gateway.send({
    op: "sig",
    t: on ? "p" : "u",
    s: state.serverId,
    c: state.channelId,
    k: kind,
  });
}

/**
 * The media attempt failed, but the voice seat stays. Drop only the pubs
 * this attempt already announced, and release camera, share, and Go Live.
 */
function retractAnnouncedMedia(): void {
  const state = useVoice.getState();
  const kinds = [...announced];
  announced.clear();
  const self = currentUserId();
  for (const kind of kinds) {
    if (self) setPub(self, kind, false);
    if (!state.serverId || !state.channelId) continue;
    deps?.gateway.send({
      op: "sig",
      t: "u",
      s: state.serverId,
      c: state.channelId,
      k: kind,
    });
  }
  awaitingLive = null;
  if (state.live && state.serverId && state.channelId) {
    applyLiveEnd(state.serverId, state.channelId, self ?? undefined);
  }
  useVoice.setState({ camera: false, sharing: false, live: false });
}

/**
 * The gateway dropped pubs and the Go Live claim on detach. The tracks
 * are still on the peer connection, so announce them again. A rejected
 * live claim takes the existing awaitingLive path. No new capture.
 */
function reannounceActive(): void {
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) return;
  const senders = seat.pc?.getSenders?.() ?? [];
  if (senders.some((sender) => sender.track?.kind === "audio")) {
    if (hasLiveTrack(localStream, "audio")) sendPub("a", true);
  }
  if (state.camera && cameraStream) sendPub("v", true);
  if (state.sharing && screenStream) {
    sendPub("s", true);
    if (hasLiveTrack(screenStream, "audio")) sendPub("sa", true);
  }
  if (!state.live) return;
  const self = currentUserId();
  if (self) {
    applyLiveStart(state.serverId, state.channelId, self);
  }
  requestLiveClaim();
}

const STREAM_TOAST =
  "Der Stream konnte nicht verbunden werden. Der Sprachkanal bleibt aktiv.";

/**
 * A failed stream renegotiation must not tear down a working voice peer.
 * Before the first answer, there is no call to keep, so the seat rolls back.
 */
function recoverNegotiation(error: unknown, mine: number): void {
  if (mine !== seat.generation) return;
  const detail = error instanceof Error ? error.message : undefined;
  logVoice("warn", "negotiation", { detail });
  noteDiagnosticEvent({
    kind: "sdp-error",
    connection: "voice",
    detail: detail ?? "sdp",
  });
  if (!seat.negotiated) {
    rollbackSeat(error);
    return;
  }
  reportStreamOnce(detail);
  void settleFailedNegotiation(mine);
}

/**
 * A failed renegotiation must return both sides to stable without closing
 * the mic. Our rejected offer is rolled back and its new tracks dropped.
 * An SFU offer we never answered is aborted so later publications flush.
 */
async function settleFailedNegotiation(mine: number): Promise<void> {
  await seat.enqueue(async () => {
    const pc = seat.pc;
    if (seat.generation !== mine || !pc) return;
    const state = seat.signalingState();
    if (state === "have-local-offer") {
      discardingPublish = true;
      try {
        try {
          await pc.setLocalDescription({ type: "rollback" });
          logVoice("warn", "rollback", { detail: "local-offer" });
        } catch (error) {
          logVoice("warn", "rollback", {
            detail: error instanceof Error ? error.message : "rollback",
          });
        }
        if (seat.generation !== mine || seat.pc !== pc) return;
        const kinds = dropUnsettledPublish();
        seat.makingOffer = false;
        // A discarded publish waits for the next user action. An ICE
        // restart that was rolled back with it still has to be offered.
        noteRestartRolledBack(seatRecoverySlot, mine);
        const restartPending =
          seatRecoverySlot.recovery?.generation === mine &&
          seatRecoverySlot.recovery.pendingIceRestart;
        seat.needOffer = restartPending;
        for (const { kind, trackId } of kinds) {
          logVoice("warn", "unpublish", { track: kind });
          seat.send({ op: "u", k: kind, t: trackId });
        }
      } finally {
        discardingPublish = false;
      }
    } else if (state === "have-remote-offer") {
      try {
        await pc.setRemoteDescription({ type: "rollback" });
        logVoice("warn", "rollback", { detail: "remote-offer" });
      } catch (error) {
        logVoice("warn", "rollback", {
          detail: error instanceof Error ? error.message : "rollback",
        });
      }
    }
    if (seat.generation !== mine || seat.pc !== pc) return;
    if (seat.sfuOfferOpen) {
      seat.sfuOfferOpen = false;
      logVoice("warn", "abort", { detail: "sfu-offer" });
      seat.send({ op: "x" });
    }
    if (seat.needOffer || owesIceRestart(seatRecoverySlot, mine)) {
      void offerIfStable(mine);
    }
    noteDiagnosticEvent({
      kind: "recovery",
      connection: "voice",
      detail: "renegotiation",
    });
  });
}

/** Remove senders added after the last successful answer. The mic stays. */
function dropUnsettledPublish(): PublishIdentity[] {
  const pc = seat.pc;
  const pending = offeredPublish;
  offeredPublish = [];
  openPublish = openPublish.filter((identity) => !pending.includes(identity));
  const discardedKinds = new Set<PublishKind>();
  for (const [sender, identity] of publisherTracks) {
    if (!pending.includes(identity)) continue;
    discardedKinds.add(identity.kind);
    pc?.removeTrack?.(sender);
    publisherTracks.delete(sender);
  }
  for (const kind of discardedKinds) {
    if (kind === "sa" || kind === "la") {
      const parent = kind === "sa" ? "s" : "l";
      if (!discardedKinds.has(parent)) stopLocalSourceAudio(parent, false);
    } else releaseDiscardedCapture(kind);
  }
  return pending;
}

/**
 * Stop a publish that never negotiated: capture, local preview, and flags.
 * Does not touch the mic or a stream that already completed an answer, and
 * does not ask for a new offer.
 */
function releaseDiscardedCapture(kind: "v" | "s" | "l"): void {
  if (kind === "l") clearLiveClaim();
  if (kind === "v") {
    cameraEpoch += 1;
    for (const pending of pendingCameraStreams) stopTracks(pending);
    pendingCameraStreams.clear();
  } else if (kind === "s") screenEpoch += 1;
  else liveEpoch += 1;
  const stream =
    kind === "v" ? cameraStream : kind === "s" ? screenStream : liveStream;
  if (kind === "v") {
    cameraStream = null;
    useVoice.setState({ camera: false, localCamera: null });
  } else if (kind === "s") {
    screenStream = null;
    useVoice.setState({ sharing: false, localScreen: null });
  } else {
    const state = useVoice.getState();
    liveStream = null;
    awaitingLive = null;
    useVoice.setState({ live: false, localLive: null });
    if (state.serverId && state.channelId) {
      applyLiveEnd(
        state.serverId,
        state.channelId,
        currentUserId() ?? undefined,
      );
    }
  }
  const self = currentUserId();
  if (kind !== "v") {
    useVoice.setState({
      sourceAudio: { ...useVoice.getState().sourceAudio, [kind]: "off" },
    });
    const audioKind = kind === "s" ? "sa" : "la";
    if (self) setPub(self, audioKind, false);
    sendPub(audioKind, false);
  }
  stopTracks(stream);
  if (self) setPub(self, kind, false);
  sendPub(kind, false);
  noteStream(kind, false);
}

function reportStreamOnce(detail?: string): void {
  logVoice("warn", "stream", { detail });
  if (streamReported) return;
  streamReported = true;
  deps?.onError?.(new Error(STREAM_TOAST));
}

async function applyRemoteDescription(
  type: "offer" | "answer",
  sdp: string,
  mine: number,
): Promise<void> {
  const pc = seat.pc;
  const current = () => seat.generation === mine && seat.pc === pc;
  if (!pc || !current()) return;
  // A duplicate/obsolete answer cannot answer an already settled offer.
  if (type === "answer" && seat.signalingState() !== "have-local-offer") return;
  if (type === "offer") {
    seat.sfuOfferOpen = true;
    const collision = seat.makingOffer || seat.signalingState() !== "stable";
    if (collision) {
      seat.needOffer = true;
      noteRestartRolledBack(seatRecoverySlot, mine);
      try {
        await pc.setLocalDescription({ type: "rollback" });
      } catch {
        // Browsers may perform implicit rollback in setRemoteDescription.
      }
      if (!current()) return;
    }
    seat.sfuOffered = true;
  }
  await pc.setRemoteDescription({ type, sdp });
  if (!current()) return;
  const queued = seat.pendingIce;
  seat.pendingIce = [];
  for (const candidate of queued) {
    try {
      await pc.addIceCandidate(candidate);
    } catch (error) {
      if (current()) {
        logVoice("warn", "ice", {
          detail: error instanceof Error ? error.message : "addIceCandidate",
          mid: candidate.sdpMid ?? undefined,
        });
        noteDiagnosticEvent({
          kind: "ice-error",
          connection: "voice",
          detail: "addIceCandidate",
        });
      }
    }
    if (!current()) return;
  }
  if (type === "offer") {
    const answer = withTunedSdp(await pc.createAnswer());
    if (!current()) return;
    await pc.setLocalDescription(answer);
    if (!current()) return;
    if (answer.sdp) {
      logVoice("info", "send-answer", { bytes: answer.sdp.length });
      seat.send({ op: "a", sdp: answer.sdp });
    }
    // The answer is on the wire. A later negotiation_failed must not send
    // `x`: an oversized answer is aborted on the server, and a late `x`
    // would roll back the next offer.
    seat.sfuOfferOpen = false;
  }
  await enqueueVideoLimits(pc);
  if (!current()) return;
  seat.negotiated = true;
  if (type === "answer") {
    openPublish = openPublish.filter(
      (identity) => !offeredPublish.includes(identity),
    );
    offeredPublish = [];
  }
  if (seat.needOffer || owesIceRestart(seatRecoverySlot, mine)) {
    void offerIfStable(mine);
  }
}

async function applyRemoteIce(candidate: IceCand, mine: number): Promise<void> {
  if (mine !== seat.generation) return;
  const pc = seat.pc;
  if (pc?.remoteDescription) {
    try {
      await pc.addIceCandidate(candidate);
    } catch (error) {
      if (mine === seat.generation && seat.pc === pc) {
        logVoice("warn", "ice", {
          detail: error instanceof Error ? error.message : "addIceCandidate",
          mid: candidate.sdpMid ?? undefined,
        });
        noteDiagnosticEvent({
          kind: "ice-error",
          connection: "voice",
          detail: "addIceCandidate",
        });
      }
    }
    return;
  }
  seat.pendingIce.push(candidate);
}

function restartSeatTransport(
  mine: number,
  options?: { allowReconnect?: boolean },
): void {
  if (seat.generation !== mine) return;
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) return;
  if (!seat.pc) return;
  const action = recoveryAction(
    seatRecoverySlot,
    mine,
    options?.allowReconnect !== false,
  );
  if (action === "reconnect") {
    scheduleSeatRebuild();
    return;
  }
  if (action === "ignore") return;
  if (action === "start") {
    beginRecovery(
      seatRecoverySlot,
      mine,
      () => seat.generation === mine,
      () => transportRecovered(seat.pc),
      () => {
        const latest = useVoice.getState();
        if (
          latest.status !== "joined" ||
          !latest.serverId ||
          !latest.channelId
        ) {
          return;
        }
        if (seat.generation !== mine) return;
        scheduleSeatRebuild();
      },
    );
  } else {
    markIceRestartPending(seatRecoverySlot, mine);
  }
  void offerIfStable(mine, { fromEvent: true, iceRestart: true });
}

function onSeatConnectionChange(mine: number): void {
  if (seat.generation !== mine) return;
  const pc = seat.pc;
  if (!pc) return;
  const ice = pc.iceConnectionState ?? "";
  const conn = pc.connectionState ?? "";
  noteTransportProgress(seatRecoverySlot, mine, ice, conn);
  if (ice === "failed" || conn === "failed") {
    clearSeatReconnectTimer();
    restartSeatTransport(mine);
    return;
  }
  if (transportRecovered(pc)) {
    seatRetry.cancel();
    clearSeatReconnectTimer();
    clearRecovery(seatRecoverySlot);
    return;
  }
  if (ice === "disconnected" || conn === "disconnected") {
    clearSeatReconnectTimer();
    seatReconnectTimer = setTimeout(() => {
      seatReconnectTimer = null;
      if (seat.generation !== mine) return;
      const latest = seat.pc;
      if (!latest || transportRecovered(latest)) return;
      const latestIce = latest.iceConnectionState ?? "";
      const latestConn = latest.connectionState ?? "";
      if (
        latestIce === "failed" ||
        latestConn === "failed" ||
        latestIce === "disconnected" ||
        latestConn === "disconnected"
      ) {
        restartSeatTransport(mine);
      }
    }, ICE_DISCONNECTED_RESTART_DELAY_MS);
  }
}

function restartWatchTransport(
  mine: number,
  options?: { allowReconnect?: boolean },
): void {
  if (watchCall.generation !== mine) return;
  const state = useVoice.getState();
  if (!state.watching || !state.watchChannelId) return;
  if (!watchCall.pc) return;
  const action = recoveryAction(
    watchRecoverySlot,
    mine,
    options?.allowReconnect !== false,
  );
  if (action === "reconnect") {
    scheduleWatchRebuild(state.watchChannelId);
    return;
  }
  if (action === "ignore") return;
  if (action === "start") {
    const channelId = state.watchChannelId;
    beginRecovery(
      watchRecoverySlot,
      mine,
      () => watchCall.generation === mine,
      () => transportRecovered(watchCall.pc),
      () => {
        const latest = useVoice.getState();
        if (!latest.watching || latest.watchChannelId !== channelId) return;
        if (watchCall.generation !== mine) return;
        scheduleWatchRebuild(channelId);
      },
    );
  } else {
    markIceRestartPending(watchRecoverySlot, mine);
  }
  void watchOfferIfStable(mine, { fromEvent: true, iceRestart: true });
}

function onWatchConnectionChange(mine: number): void {
  if (watchCall.generation !== mine) return;
  const pc = watchCall.pc;
  if (!pc) return;
  const ice = pc.iceConnectionState ?? "";
  const conn = pc.connectionState ?? "";
  noteTransportProgress(watchRecoverySlot, mine, ice, conn);
  if (ice === "failed" || conn === "failed") {
    clearWatchReconnectTimer();
    restartWatchTransport(mine);
    return;
  }
  if (transportRecovered(pc)) {
    watchRetry.cancel();
    clearWatchReconnectTimer();
    clearRecovery(watchRecoverySlot);
    return;
  }
  if (ice === "disconnected" || conn === "disconnected") {
    clearWatchReconnectTimer();
    watchReconnectTimer = setTimeout(() => {
      watchReconnectTimer = null;
      if (watchCall.generation !== mine) return;
      const latest = watchCall.pc;
      if (!latest || transportRecovered(latest)) return;
      const latestIce = latest.iceConnectionState ?? "";
      const latestConn = latest.connectionState ?? "";
      if (
        latestIce === "failed" ||
        latestConn === "failed" ||
        latestIce === "disconnected" ||
        latestConn === "disconnected"
      ) {
        restartWatchTransport(mine);
      }
    }, ICE_DISCONNECTED_RESTART_DELAY_MS);
  }
}

function onMediaFrame(frame: MediaServerFrame): void {
  const mine = seat.generation;
  if (frame.op === "ok") {
    seatMediaVersion = frame.v ?? 1;
    useVoice.setState({ sourceWatchSupported: seatMediaVersion >= 2 });
    seat.accept();
    if (seatMediaVersion >= 2) {
      for (const [userId, subscriptions] of Object.entries(
        useVoice.getState().sourceSubscriptions,
      )) {
        for (const kind of ["s", "l"] as const)
          if (subscriptions[kind])
            seat.send({ op: "w", u: userId, k: kind, on: true });
      }
    }
    const captures = [
      ["s", screenStream],
      ["l", liveStream],
    ] as const;
    for (const [kind, stream] of captures) {
      if (!hasLiveTrack(stream, "audio")) continue;
      if (seatMediaVersion >= 2) void publishLocal(kind, stream!);
      else {
        stream!.getAudioTracks().forEach((track) => track.stop());
        useVoice.setState({
          sourceAudio: {
            ...useVoice.getState().sourceAudio,
            [kind]: "unsupported",
          },
        });
      }
    }
    return;
  }
  if (frame.op === "err") {
    logVoice("warn", "media-error", { op: frame.op, code: frame.e });
    if (frame.e === "ice_failed") {
      noteDiagnosticEvent({
        kind: "ice-error",
        connection: "voice",
        detail: "ice_failed",
      });
      restartSeatTransport(mine, { allowReconnect: false });
      return;
    }
    if (frame.e === "unavailable") {
      if (seatRetry.active) {
        stopPeer(true);
        scheduleSeatRebuild();
        return;
      }
      deps?.onError?.(new Error("Kein freier Sprachplatz."));
      // Drop this attempt, its queued signaling, and the local capture.
      // Unpublish what this attempt already announced, then close the
      // peer. The gateway seat stays. A later unauthorized belongs to
      // the generation we just closed.
      retractAnnouncedMedia();
      stopPeer();
      return;
    }
    if (frame.e === "forbidden") {
      if (isLiveNonce(frame.lc)) {
        if (!useVoice.getState().live) return;
        // A delayed old withdrawal cannot stop a newly acknowledged claim.
        if (liveClaimNonce && frame.lc !== liveClaimNonce) return;
        if (frame.lc === withdrawnLiveNonce) return;
        withdrawnLiveNonce = frame.lc;
        liveRecoveryDeadline ??= Date.now() + 10000;
        // SFU forwarding is already stopped. Keep only local capture while a
        // fresh Gateway acknowledgement is pending, within one fixed budget.
        requestLiveClaim();
        return;
      }
      if (useVoice.getState().live) stopLocalVideo("l");
      deps?.onError?.(new ApiError("forbidden", 0, errorMessage("forbidden")));
      return;
    }
    if (frame.e === "unauthorized") {
      rollbackSeat(
        new ApiError("unauthenticated", 0, errorMessage("unauthenticated")),
      );
      return;
    }
    if (!seat.negotiated) {
      noteDiagnosticEvent({
        kind: "sdp-error",
        connection: "voice",
        detail: frame.e,
      });
      rollbackSeat(new ApiError("bad_request", 0, errorMessage("bad_request")));
      return;
    }
    noteDiagnosticEvent({
      kind: "sdp-error",
      connection: "voice",
      detail: frame.e,
    });
    reportStreamOnce(frame.e);
    void settleFailedNegotiation(mine);
    return;
  }
  if ((frame.op === "a" || frame.op === "o") && frame.sdp) {
    const type = frame.op === "a" ? "answer" : "offer";
    logVoice("info", type === "offer" ? "recv-offer" : "recv-answer", {
      bytes: frame.sdp.length,
    });
    void seat
      .enqueue(() => applyRemoteDescription(type, frame.sdp, mine))
      .catch((error) => recoverNegotiation(error, mine));
    return;
  }
  if (frame.op === "i" && frame.ice) {
    void applyRemoteIce(
      { candidate: frame.ice, sdpMid: frame.mid ?? null },
      mine,
    );
  }
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
  soundOnJoin =
    prev.serverId !== input.serverId || prev.channelId !== input.channelId;
  unlockCallSounds();
  const muted = prev.status === "joined" && (prev.muted || prev.deafened);
  const deafened = prev.status === "joined" && prev.deafened;
  setCallSoundsDeafened(deafened);
  if (prev.status !== "joined") preDeafenMuted = false;
  const dropWatch =
    prev.watchServerId === input.serverId &&
    prev.watchChannelId === input.channelId;
  resetDiagnostics();
  streamReported = false;
  if (
    prev.status === "joined" &&
    prev.serverId &&
    prev.channelId &&
    (prev.serverId !== input.serverId || prev.channelId !== input.channelId)
  ) {
    vacateSelf(prev.serverId, prev.channelId);
    deps?.gateway.send({
      op: "sig",
      t: "l",
      s: prev.serverId,
      c: prev.channelId,
    });
  }
  stopPeer();
  if (dropWatch) {
    stopWatching();
  } else if (prev.watching && watchCall.pc) {
    attachWatchDiagnostics(watchCall.generation);
  }
  const self = userId;
  useVoice.setState({
    status: "joined",
    serverId: input.serverId,
    channelId: input.channelId,
    channelName: input.channelName,
    muted,
    deafened,
    camera: false,
    sharing: false,
    live: false,
    localCamera: null,
    localScreen: null,
    localLive: null,
    sourceSubscriptions: {},
    sourceWatchSupported: false,
    remote: {},
    participants: { [self]: { pubs: [] } },
  });
  applyVoiceJoin(input.serverId, self, input.channelId, {
    muted,
    deafened,
  });
  awaitingJoin = { serverId: input.serverId, channelId: input.channelId };
  deps?.gateway.send({
    op: "sig",
    t: "j",
    s: input.serverId,
    c: input.channelId,
  });
  // The join frame has no mute fields. Sync flags in socket order before media work.
  if (muted) sendFlag("m", input.serverId, input.channelId, true);
  if (deafened) sendFlag("d", input.serverId, input.channelId, true);
  applyPlayback();
  void startPeer(input.serverId, input.channelId);
}

export function leaveVoice(): void {
  const state = useVoice.getState();
  const hadJoined = !awaitingJoin && state.status === "joined";
  soundOnJoin = false;
  stopCallSounds();
  if (hadJoined && !state.deafened) playCallSound("leave");
  setCallSoundsDeafened(false);
  const serverId = state.serverId;
  const channelId = state.channelId;
  awaitingJoin = null;
  awaitingLive = null;
  republishOnJoin = false;
  if (state.live && serverId && channelId) {
    applyLiveEnd(serverId, channelId, currentUserId() ?? undefined);
  }
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...idle,
    watching: state.watching,
    watchServerId: state.watchServerId,
    watchChannelId: state.watchChannelId,
    watchChannelName: state.watchChannelName,
    watchPublisherId: state.watchPublisherId,
    watchStream: state.watchStream,
    playbackBlocked: blockedPlayback.size > 0,
  });
  applyPlayback();
  if (serverId && channelId) {
    deps?.gateway.send({
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
  if (!deafened) preDeafenMuted = muted;
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
  if (deafened) preDeafenMuted = state.muted;
  const muted = deafened || preDeafenMuted;
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

/**
 * Go Live badge flips immediately; display capture and SFU publish follow.
 * One live track per voice channel. Needs `go_live` on the server.
 */
export function toggleGoLive(): void {
  ensureBound();
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) {
    return;
  }
  if (state.live) {
    awaitingLive = null;
    stopLocalVideo("l");
    return;
  }
  const self = currentUserId();
  if (!self) return;
  const holder = liveOf(
    useVoiceRoster.getState().live,
    state.serverId,
    state.channelId,
  );
  if (holder && holder !== self) {
    return;
  }
  useVoice.setState({ live: true });
  applyLiveStart(state.serverId, state.channelId, self);
  void startLocalVideo("l");
  requestLiveClaim();
}

/**
 * Subscribe to a live track without publishing. No mic permission prompt.
 */
export function watchLive(input: {
  serverId: string;
  channelId: string;
  channelName: string;
}): void {
  installDiagnosticsLogoutReset();
  ensureBound();
  const state = useVoice.getState();
  if (
    state.status === "joined" &&
    state.serverId === input.serverId &&
    state.channelId === input.channelId
  ) {
    return;
  }
  if (
    state.watching &&
    state.watchServerId === input.serverId &&
    state.watchChannelId === input.channelId
  ) {
    return;
  }
  clearWatchPublisherTimer();
  useVoice.setState({
    watching: true,
    watchServerId: input.serverId,
    watchChannelId: input.channelId,
    watchChannelName: input.channelName,
    watchPublisherId: liveOf(
      useVoiceRoster.getState().live,
      input.serverId,
      input.channelId,
    ),
    watchStream: null,
  });
  void startWatchPeer(input.channelId);
}

export function stopWatching(): void {
  clearWatchPublisherTimer();
  stopWatchPeer();
  useVoice.setState({
    watching: false,
    watchServerId: null,
    watchChannelId: null,
    watchChannelName: null,
    watchPublisherId: null,
    watchStream: null,
  });
}

export function resetVoiceForTests(): void {
  soundOnJoin = false;
  stopCallSounds();
  setCallSoundsDeafened(false);
  preDeafenMuted = false;
  clearWatchPublisherTimer();
  resetDiagnostics();
  streamReported = false;
  watchReported = false;
  awaitingJoin = null;
  awaitingLive = null;
  republishOnJoin = false;
  seat.pendingIce = [];
  audioCommitChain = Promise.resolve();
  cameraCommitChain = Promise.resolve();
  clearSeatReconnectTimer();
  clearWatchReconnectTimer();
  stopWatchPeer();
  stopPeer();
  remoteAudio = null;
  sourceAudio.clear();
  watchAudio.clear();
  blockedPlayback.clear();
  seat = new MediaPeer();
  watchCall = new MediaPeer();
  resetVideoLimitQueue();
  useVoice.setState({ ...idle });
  deps = null;
  bound = false;
  onMediaSettingsChange(null);
}

function endedListener(kind: "v" | "s" | "l"): () => void {
  return () => {
    const state = useVoice.getState();
    if (kind === "v" && state.camera) stopLocalVideo("v");
    if (kind === "s" && state.sharing) stopLocalVideo("s");
    if (kind === "l" && state.live) stopLocalVideo("l");
  };
}

function bindEnded(stream: MediaStream, kind: "v" | "s" | "l"): void {
  const onEnded = endedListener(kind);
  for (const track of stream.getVideoTracks()) {
    track.addEventListener("ended", onEnded);
  }
}

function videoEpoch(kind: "v" | "s" | "l"): number {
  if (kind === "v") return cameraEpoch;
  if (kind === "s") return screenEpoch;
  return liveEpoch;
}

function bumpVideoEpoch(kind: "v" | "s" | "l"): number {
  if (kind === "v") return ++cameraEpoch;
  if (kind === "s") return ++screenEpoch;
  return ++liveEpoch;
}

async function startLocalVideo(kind: "v" | "s" | "l"): Promise<void> {
  const epoch = bumpVideoEpoch(kind);
  const mine = seat.generation;
  // A display capture owns its video and optional browser-selected audio.
  let stream: MediaStream;
  try {
    stream = await captureVideo(kind);
  } catch (error) {
    if (
      isOverconstrainedError(error) &&
      seat.generation === mine &&
      videoEpoch(kind) === epoch
    ) {
      deps?.onError?.(
        new Error(
          kind === "v"
            ? "Die Kamera unterstützt das Streamprofil nicht."
            : "Die Bildschirmfreigabe unterstützt das Streamprofil nicht.",
        ),
      );
    }
    if (kind === "v" && cameraEpoch === epoch) {
      useVoice.setState({ camera: false });
    }
    if (kind === "s" && screenEpoch === epoch) {
      useVoice.setState({ sharing: false });
    }
    if (kind === "l" && liveEpoch === epoch) {
      const state = useVoice.getState();
      awaitingLive = null;
      useVoice.setState({ live: false, localLive: null });
      if (state.serverId && state.channelId) {
        applyLiveEnd(
          state.serverId,
          state.channelId,
          currentUserId() ?? undefined,
        );
      }
    }
    return;
  }
  pendingDisplayStreams.delete(stream);
  if (seat.generation !== mine || videoEpoch(kind) !== epoch) {
    stopTracks(stream);
    return;
  }
  const self = currentUserId();
  if (kind !== "v") {
    const sharingAudio = hasLiveTrack(stream, "audio");
    if (sharingAudio && seatMediaVersion !== null && seatMediaVersion < 2)
      stream.getAudioTracks().forEach((track) => track.stop());
    useVoice.setState({
      sourceAudio: {
        ...useVoice.getState().sourceAudio,
        [kind]: sharingAudio
          ? seatMediaVersion !== null && seatMediaVersion < 2
            ? "unsupported"
            : "sharing"
          : useMediaSettings.getState().shareSourceAudio
            ? "unavailable"
            : "off",
      },
    });
    for (const track of stream.getAudioTracks()) {
      hintTrack(track, "music");
      track.addEventListener("ended", () => {
        const active = kind === "s" ? screenStream : liveStream;
        if (active === stream) stopLocalSourceAudio(kind);
      });
    }
  }
  bindEnded(stream, kind);
  if (kind === "v") {
    stopTracks(cameraStream);
    cameraStream = stream;
    useVoice.setState({ camera: true, localCamera: stream });
  } else if (kind === "s") {
    stopTracks(screenStream);
    screenStream = stream;
    stream.getVideoTracks().forEach((track) => hintTrack(track, "detail"));
    useVoice.setState({ sharing: true, localScreen: stream });
  } else {
    stopTracks(liveStream);
    liveStream = stream;
    stream.getVideoTracks().forEach((track) => hintTrack(track, "detail"));
    useVoice.setState({ live: true, localLive: stream });
  }
  noteStreamProfileApply(kind === "v" ? "camera" : "screen", "idle");
  if (self) setPub(self, kind, true);
  noteStream(kind, true);
  // Yield so the local tile paints before addTrack / offer.
  await Promise.resolve();
  if (seat.generation !== mine || videoEpoch(kind) !== epoch) {
    return;
  }
  await publishLocal(kind, stream);
}

function stopLocalVideo(kind: "v" | "s" | "l"): void {
  for (const [pending, heldKind] of pendingDisplayStreams) {
    if (heldKind === kind) {
      stopTracks(pending);
      pendingDisplayStreams.delete(pending);
    }
  }
  const state = useVoice.getState();
  if (kind === "v" && !state.camera && !state.localCamera) return;
  if (kind === "s" && !state.sharing && !state.localScreen) return;
  if (kind === "l" && !state.live && !state.localLive) return;
  if (kind === "l") {
    clearLiveClaim();
    awaitingLive = null;
  }
  if (kind === "v") {
    cameraEpoch += 1;
    for (const pending of pendingCameraStreams) stopTracks(pending);
    pendingCameraStreams.clear();
  } else if (kind === "s") screenEpoch += 1;
  else liveEpoch += 1;
  const stream =
    kind === "v" ? cameraStream : kind === "s" ? screenStream : liveStream;
  const senders = seat.pc?.getSenders?.() ?? [];
  if (kind === "v") {
    cameraStream = null;
    useVoice.setState({ camera: false, localCamera: null });
  } else if (kind === "s") {
    screenStream = null;
    useVoice.setState({ sharing: false, localScreen: null });
  } else {
    liveStream = null;
    useVoice.setState({ live: false, localLive: null });
    if (state.serverId && state.channelId) {
      applyLiveEnd(
        state.serverId,
        state.channelId,
        currentUserId() ?? undefined,
      );
    }
  }
  const self = currentUserId();
  if (self) setPub(self, kind, false);
  sendPub(kind, false);
  for (const track of stream?.getTracks() ?? []) {
    const sender = senders.find((item) => item.track === track);
    if (sender) {
      const identity = publisherTracks.get(sender);
      seat.send({
        op: "u",
        k: identity?.kind ?? kind,
        t: identity?.trackId ?? track.id,
      });
      publisherTracks.delete(sender);
      seat.pc?.removeTrack?.(sender);
    } else {
      seat.send({
        op: "u",
        k:
          track.kind === "audio" && kind !== "v"
            ? kind === "s"
              ? "sa"
              : "la"
            : kind,
        t: track.id,
      });
    }
    openPublish = openPublish.filter(
      (item) =>
        item.kind !== kind &&
        item.kind !== (kind === "s" ? "sa" : kind === "l" ? "la" : "v"),
    );
    track.stop();
  }
  if (kind !== "v") {
    useVoice.setState({
      sourceAudio: { ...useVoice.getState().sourceAudio, [kind]: "off" },
    });
    const audioKind = kind === "s" ? "sa" : "la";
    if (self) setPub(self, audioKind, false);
    sendPub(audioKind, false);
  }
  if (seat.pc) void enqueueVideoLimits(seat.pc);
  if (kind === "v") noteStreamProfileApply("camera", "idle");
  else if (!screenStream && !liveStream) {
    noteStreamProfileApply("screen", "idle");
  }
  seat.needOffer = true;
  noteStream(kind, false);
  void offerIfStable(seat.generation);
}

async function publishLocal(
  kind: "v" | "s" | "l",
  stream: MediaStream,
): Promise<void> {
  if (!seat.pc) return;
  if (kind === "l" && !liveClaimNonce) return;
  const tracks = [
    ...stream.getVideoTracks(),
    ...(kind === "v" || seatMediaVersion === null || seatMediaVersion < 2
      ? []
      : stream
          .getAudioTracks()
          .filter((track) => track.readyState !== "ended")),
  ];
  if (stream.getVideoTracks().length === 0) return;
  const pc = seat.pc;
  const mine = seat.generation;
  const epoch = videoEpoch(kind);
  logVoice("info", "publish", { track: kind });
  for (const track of tracks) {
    const publishKind: PublishKind =
      track.kind === "audio" ? (kind === "s" ? "sa" : "la") : kind;
    // addTrack may reuse a stopped sender/transceiver. Reserve by the new
    // MSID identity, rather than treating the sender object as a new publish.
    let sender = pc.getSenders?.().find((sender) => sender.track === track);
    const reserved = videoSenders.get(publishKind);
    const transceiver = pc
      .getTransceivers?.()
      .find((item) => item.sender === reserved);
    if (
      !sender &&
      reserved?.replaceTrack &&
      !reserved.track &&
      !transceiver?.stopped
    ) {
      await reserved.replaceTrack(track);
      if (
        seat.generation !== mine ||
        seat.pc !== pc ||
        videoEpoch(kind) !== epoch
      ) {
        if (reserved.track === track) pc.removeTrack?.(reserved);
        return;
      }
      if (transceiver?.direction === "recvonly")
        transceiver.direction = "sendrecv";
      else if (transceiver?.direction === "inactive")
        transceiver.direction = "sendonly";
      sender = reserved;
    }
    sender ??= pc.addTrack?.(track, stream) || undefined;
    const identity: PublishIdentity = (sender &&
      publisherTracks.get(sender)) ?? { kind: publishKind, trackId: track.id };
    if (sender) {
      publisherTracks.set(sender, identity);
      videoSenders.set(publishKind, sender);
    }
    if (!openPublish.includes(identity)) openPublish.push(identity);
  }
  void enqueueVideoLimits(pc);
  const self = currentUserId();
  if (self) setPub(self, kind, true);
  if (kind !== "l") sendPub(kind, true);
  if (
    kind !== "v" &&
    seatMediaVersion !== null &&
    seatMediaVersion >= 2 &&
    hasLiveTrack(stream, "audio")
  ) {
    const audioKind = kind === "s" ? "sa" : "la";
    if (self) setPub(self, audioKind, true);
    sendPub(audioKind, true);
  }
  void applySendBitrate();
  seat.needOffer = true;
  await offerIfStable(seat.generation);
}

function stopLocalSourceAudio(kind: "s" | "l", negotiate = true): void {
  const stream = kind === "s" ? screenStream : liveStream;
  const audioKind = kind === "s" ? "sa" : "la";
  for (const track of stream?.getAudioTracks() ?? []) {
    const sender = seat.pc?.getSenders?.().find((item) => item.track === track);
    const identity = sender && publisherTracks.get(sender);
    seat.send({ op: "u", k: audioKind, t: identity?.trackId ?? track.id });
    if (sender) {
      publisherTracks.delete(sender);
      seat.pc?.removeTrack?.(sender);
    }
    track.stop();
  }
  openPublish = openPublish.filter((item) => item.kind !== audioKind);
  offeredPublish = offeredPublish.filter((item) => item.kind !== audioKind);
  const self = currentUserId();
  if (self) setPub(self, audioKind, false);
  sendPub(audioKind, false);
  useVoice.setState({
    sourceAudio: { ...useVoice.getState().sourceAudio, [kind]: "ended" },
  });
  if (negotiate) {
    seat.needOffer = true;
    void offerIfStable(seat.generation);
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
    seat.send({ op: "w", u: userId, k: kind, on });
  if (!on) {
    dropRemote(userId, kind);
    clearSourcePlayback("voice", userId, kind);
  } else {
    reattachReceived(userId, kind);
    for (const [track, received] of receivedSourceAudio) {
      if (
        received.userId === userId &&
        received.kind === kind &&
        track.readyState !== "ended"
      )
        attachSourceAudio(track, received.stream, "voice");
    }
  }
}

function clearSourcePlayback(
  role: "voice" | "watch",
  userId?: string,
  kind?: "s" | "l",
): void {
  for (const [track, entry] of sourceAudio) {
    if (
      entry.role !== role ||
      (userId && entry.userId !== userId) ||
      (kind && entry.kind !== kind)
    )
      continue;
    entry.el.pause?.();
    entry.el.srcObject = null;
    blockedPlayback.delete(entry.el);
    sourceAudio.delete(track);
  }
  updatePlaybackBlocked();
}

function attachSourceAudio(
  track: MediaStreamTrack,
  stream: MediaStream | undefined,
  role: "voice" | "watch",
): boolean {
  // A reused browser receiver keeps its original track.id. The current parent
  // stream MSID identifies both the publisher and its source after replacement.
  const parent = parseRemoteStreamId(stream?.id ?? "");
  if (stream && /:a(?:[-:].*)?$/.test(stream.id)) return false;
  const parsed =
    parent && (parent.k === "s" || parent.k === "l")
      ? { userId: parent.userId, k: parent.k === "s" ? "sa" : "la" }
      : (parent ?? parseRemoteStreamId(track.id));
  if (!parsed || (parsed.k !== "sa" && parsed.k !== "la")) return false;
  if (remoteMix?.getTracks().includes(track)) remoteMix.removeTrack(track);
  const voiceElement = watchAudio.get(track);
  if (voiceElement) {
    voiceElement.pause?.();
    voiceElement.srcObject = null;
    blockedPlayback.delete(voiceElement);
    watchAudio.delete(track);
  }
  const state = useVoice.getState();
  const kind = parsed.k === "sa" ? "s" : "l";
  receivedAudioSources.set(track, {
    role,
    source: kind === "s" ? "screen-audio" : "live-audio",
  });
  if (role === "voice")
    receivedSourceAudio.set(track, { stream, userId: parsed.userId, kind });
  const held = sourceAudio.get(track);
  const sameSource =
    held?.role === role &&
    held.userId === parsed.userId &&
    held.kind === kind &&
    held.el.srcObject === stream;
  if (held && !sameSource)
    clearSourcePlayback(held.role, held.userId, held.kind);
  const allowed =
    role === "voice"
      ? state.sourceSubscriptions[parsed.userId]?.[kind]
      : kind === "l" &&
        state.watching &&
        state.watchPublisherId === parsed.userId;
  if (
    !allowed ||
    parsed.userId === currentUserId() ||
    typeof Audio === "undefined"
  )
    return true;
  if (sameSource) return true;
  clearSourcePlayback(role, parsed.userId, kind);
  for (const [previous, source] of receivedSourceAudio) {
    if (
      previous !== track &&
      source.userId === parsed.userId &&
      source.kind === kind
    )
      receivedSourceAudio.delete(previous);
  }
  const el = new Audio();
  el.autoplay = true;
  el.setAttribute("playsinline", "true");
  el.srcObject =
    stream ??
    (typeof MediaStream === "undefined" ? null : new MediaStream([track]));
  el.setAttribute("data-source-audio", kind);
  el.setAttribute("data-publisher", parsed.userId);
  el.setAttribute("data-connection", role);
  sourceAudio.set(track, { el, role, userId: parsed.userId, kind });
  track.addEventListener("ended", () => {
    if (sourceAudio.get(track)?.el !== el) return;
    receivedSourceAudio.delete(track);
    receivedAudioSources.delete(track);
    sourceAudio.delete(track);
    el.pause?.();
    el.srcObject = null;
    blockedPlayback.delete(el);
    updatePlaybackBlocked();
  });
  applyPlayback();
  playAudio(el);
  return true;
}

function forgetSourceAudioReceiver(track: MediaStreamTrack): void {
  const held = sourceAudio.get(track);
  if (held) clearSourcePlayback(held.role, held.userId, held.kind);
  receivedSourceAudio.delete(track);
}

function parseIncomingVideo(
  track: MediaStreamTrack,
  stream?: MediaStream,
): { userId: string; k: "v" | "s" | "l" } | null {
  const parsed =
    parseRemoteStreamId(stream?.id ?? "") ?? parseRemoteStreamId(track.id);
  return parsed && (parsed.k === "v" || parsed.k === "s" || parsed.k === "l")
    ? { userId: parsed.userId, k: parsed.k }
    : null;
}

function attachIncoming(track: MediaStreamTrack, stream?: MediaStream): void {
  if (track.kind === "audio") {
    if (attachSourceAudio(track, stream, "voice")) return;
    forgetSourceAudioReceiver(track);
    receivedAudioSources.set(track, { role: "voice", source: "voice" });
    if (typeof MediaStream === "undefined") {
      if (stream) (deps?.attachRemote ?? defaultAttachRemote)(stream);
      return;
    }
    if (!remoteMix) remoteMix = new MediaStream();
    if (!remoteMix.getTracks().includes(track)) {
      remoteMix.addTrack(track);
    }
    (deps?.attachRemote ?? defaultAttachRemote)(remoteMix);
    return;
  }
  const parsed = parseIncomingVideo(track, stream);
  if (!parsed) {
    logVoice("warn", "track", { track: track.kind, detail: "untagged" });
    return;
  }
  logVoice("info", "track", { track: parsed.k, peer: parsed.userId });
  const attached = stream ?? new MediaStream([track]);
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

async function offerIfStable(
  mine: number,
  opts?: { initial?: boolean; fromEvent?: boolean; iceRestart?: boolean },
): Promise<void> {
  const epoch = seatEpoch;
  const peer = seat;
  const live = () =>
    seatEpoch === epoch &&
    seat === peer &&
    seat.generation === mine &&
    !!seat.pc;
  if (discardingPublish) return;
  await peer.enqueue(async () => {
    if (discardingPublish || !live()) return;
    if (opts?.initial && seat.sfuOffered) return;
    const restart = wantsIceRestart(seatRecoverySlot, mine, opts?.iceRestart);
    const sticky = restart || (!opts?.initial && !opts?.fromEvent);
    if (seat.makingOffer || seat.signalingState() !== "stable") {
      // Sticky only for explicit publish/unpublish. negotiationneeded
      // fires again once we are stable (W3C perfect negotiation).
      // A deferred ICE restart keeps its flag until createOffer runs.
      if (sticky) seat.needOffer = true;
      if (restart) markIceRestartPending(seatRecoverySlot, mine);
      return;
    }
    seat.makingOffer = true;
    seat.needOffer = false;
    try {
      if (seat.signalingState() !== "stable") {
        if (sticky) seat.needOffer = true;
        if (restart) markIceRestartPending(seatRecoverySlot, mine);
        return;
      }
      if (opts?.initial && seat.sfuOffered) return;
      const pc = seat.pc;
      if (!pc) return;
      preferOpus(pc);
      preferVp8(pc);
      const offer = withTunedSdp(
        await pc.createOffer(restart ? { iceRestart: true } : undefined),
      );
      if (!live()) return;
      if (seat.signalingState() !== "stable") {
        if (sticky) seat.needOffer = true;
        if (restart) markIceRestartPending(seatRecoverySlot, mine);
        return;
      }
      await pc.setLocalDescription(offer);
      if (!live()) return;
      const localSdp = pc.localDescription?.sdp ?? offer.sdp;
      const bindings = publishedTrackIds(localSdp ?? "");
      offeredPublish = [];
      for (const [sender, identity] of publisherTracks) {
        if (!openPublish.includes(identity)) continue;
        const transceiver = pc
          .getTransceivers?.()
          .find((item) => item.sender === sender);
        const trackId = transceiver?.mid
          ? bindings.get(transceiver.mid)
          : undefined;
        if (pc.getTransceivers && !trackId) {
          // Capture may have arrived after createOffer. It belongs to the next
          // offer; an answer for this offer must not settle that publication.
          seat.needOffer = true;
          continue;
        }
        identity.trackId = trackId ?? identity.trackId;
        if (
          (identity.kind === "l" || identity.kind === "la") &&
          !liveClaimNonce
        )
          continue;
        offeredPublish.push(identity);
        seat.send({
          op: "p",
          k: identity.kind,
          t: identity.trackId,
          ...(identity.kind === "l" || identity.kind === "la"
            ? { lc: liveClaimNonce! }
            : {}),
        });
      }
      if (restart) noteRestartOfferCreated(seatRecoverySlot, mine);
      if (!localSdp) return;
      logVoice("info", "send-offer", { bytes: localSdp.length });
      seat.send({ op: "o", sdp: localSdp });
    } catch (error) {
      if (restart && live()) markIceRestartPending(seatRecoverySlot, mine);
      if (live()) recoverNegotiation(error, mine);
    } finally {
      if (live()) seat.makingOffer = false;
    }
  });
}

function scheduleSeatRebuild(): void {
  const state = useVoice.getState();
  if (state.status !== "joined" || !state.serverId || !state.channelId) return;
  const mine = seat.generation;
  seatRetry.schedule(
    () => {
      const current = useVoice.getState();
      if (
        seat.generation !== mine ||
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
      ),
  );
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
  const mine = seat.generation + 1;
  const resumeCamera = useVoice.getState().camera;
  const resumeShare = useVoice.getState().sharing;
  const resumeLive = useVoice.getState().live;
  stopPeer(recovering);
  seat.generation = mine;
  if (!recovering) {
    if (resumeCamera) useVoice.setState({ camera: true });
    if (resumeShare) useVoice.setState({ sharing: true });
    if (resumeLive) useVoice.setState({ live: true });
  }
  const fetchTicket = deps?.fetchTicket ?? requestMediaTicket;
  const openMedia = deps?.openMedia ?? openMediaSocket;
  const createPeer = deps?.createPeer ?? defaultCreatePeer;
  const getUserMedia = deps?.getUserMedia ?? defaultGetUserMedia;

  let iceServers: IceServer[];
  try {
    const ticket = await fetchTicket(channelId);
    if (seat.generation !== mine) return;
    iceServers = ticket.ice_servers ?? [];
    const socket = openMedia(mediaWsUrl(ticket.media_path));
    seat.bind(
      socket,
      (frame) => {
        if (seat.generation === mine) onMediaFrame(frame);
      },
      () => {
        if (seat.generation !== mine) return;
        const state = useVoice.getState();
        if (state.status !== "joined" || !state.serverId || !state.channelId) {
          return;
        }
        scheduleSeatRebuild();
      },
    );
    seat.send({ op: "j", tk: ticket.ticket, v: 2 });
  } catch (error) {
    if (seat.generation !== mine) return;
    if (recovering && retryableTicketError(error)) {
      stopPeer(true);
      scheduleSeatRebuild();
      return;
    }
    rollbackSeat(error);
    return;
  }

  if (seat.generation !== mine) return;
  const pc = createPeer(iceServers);
  seat.pc = pc;
  clearSeatReconnectTimer();
  clearRecovery(seatRecoverySlot);
  attachSeatDiagnostics(mine);

  pc.onnegotiationneeded = () => {
    if (seat.generation !== mine) return;
    void offerIfStable(mine, { fromEvent: true });
  };
  pc.oniceconnectionstatechange = () => {
    noteIceState(pc, "voice", mine, () => seat.generation);
    onSeatConnectionChange(mine);
  };
  pc.onconnectionstatechange = () => {
    onSeatConnectionChange(mine);
  };
  pc.onicecandidate = (event) => {
    if (seat.generation !== mine) return;
    if (!event.candidate?.candidate) return;
    seat.send({
      op: "i",
      ice: event.candidate.candidate,
      ...(event.candidate.sdpMid ? { mid: event.candidate.sdpMid } : {}),
    });
  };
  pc.ontrack = (event) => {
    if (seat.generation !== mine) return;
    attachIncoming(event.track, event.streams[0]);
  };

  if (hasLiveTrack(localStream, "audio")) {
    for (const track of localStream!.getAudioTracks())
      pc.addTrack?.(track, localStream!);
    applyLocalAudio();
    announced.add("a");
    deps?.gateway.send({
      op: "sig",
      t: "p",
      s: serverId,
      c: channelId,
      k: "a",
    });
    await applySendBitrate().catch(() => undefined);
  } else if (!recovering) {
    const micRequest = ++micEpoch;
    try {
      const { raw: stream, processor: insert } = await acquireMic(
        getUserMedia,
        () =>
          seat.generation === mine && micEpoch === micRequest && seat.pc === pc,
      );
      const send = insert.stream;
      if (seat.generation !== mine) {
        insert.dispose();
        stopTracks(stream);
        return;
      }
      if (micEpoch !== micRequest) {
        insert.dispose();
        stopTracks(stream);
        // A newer refreshMic owns capture; continue without this stream.
      } else {
        disableAudio(stream);
        pendingMicRaw.add(stream);
        disableAudio(send);
        await enqueueAudioCommit(async () => {
          if (
            seat.generation !== mine ||
            micEpoch !== micRequest ||
            seat.pc !== pc
          ) {
            insert?.dispose();
            stopTracks(stream);
            pendingMicRaw.delete(stream);
            return;
          }
          if (!insert.usable()) {
            discardFailedMicCandidate(stream, insert, true);
            return;
          }
          rawMicStream = stream;
          localStream = send;
          activeMicGain = insert;
          noteAudioProcessing(insert.info);
          pendingMicRaw.delete(stream);
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
          for (const track of send.getTracks()) {
            pc.addTrack?.(track, send);
          }
          try {
            await applySendBitrate();
          } catch {
            // bitrate is best-effort
          }
          if (
            seat.generation !== mine ||
            activeMicGain !== insert ||
            !insert.usable()
          )
            return;
          preferOpus(pc);
          preferVp8(pc);
          announceMicrophone();
        });
      }
    } catch (error) {
      if (seat.generation === mine) {
        deps?.onError?.(error);
        noteAudioProcessing(null);
        const info = useAudioProcessing.getState();
        noteAudioProcessing({
          ...info,
          requested: useMediaSettings.getState().processingMode,
          message: "Mikrofon konnte nicht aktiviert werden; du hörst weiter zu",
        });
      }
    }
  }

  if (seat.generation !== mine) return;
  if (!hasLiveTrack(localStream, "audio"))
    pc.addTransceiver?.("audio", { direction: "recvonly" });

  if (seat.generation !== mine) return;
  const pending = useVoice.getState();
  if (pending.localCamera) {
    await publishLocal("v", pending.localCamera);
  }
  if (pending.localScreen) {
    await publishLocal("s", pending.localScreen);
  }
  if (pending.localLive) {
    await publishLocal("l", pending.localLive);
  }
  if (
    !recovering &&
    (pending.camera || resumeCamera) &&
    !useVoice.getState().localCamera
  ) {
    void startLocalVideo("v");
  }
  if (
    !recovering &&
    (pending.sharing || resumeShare) &&
    !useVoice.getState().localScreen
  ) {
    void startLocalVideo("s");
  }
  if (
    !recovering &&
    (pending.live || resumeLive) &&
    !useVoice.getState().localLive
  ) {
    void startLocalVideo("l");
  }
  if (seat.generation !== mine) return;
  await offerIfStable(mine, { initial: true });
}

function stopWatchPeer(preserveRetry = false): void {
  watchRetry.cancel(!preserveRetry);
  watchReported = false;
  watchEpoch += 1;
  clearWatchReconnectTimer();
  clearRecovery(watchRecoverySlot);
  detachDiagnostics("watch");
  watchCall.close();
  clearSourcePlayback("watch");
  for (const [track, source] of receivedAudioSources)
    if (source.role === "watch") receivedAudioSources.delete(track);
  for (const el of watchAudio.values()) {
    el.pause?.();
    el.srcObject = null;
    blockedPlayback.delete(el);
  }
  watchAudio.clear();
  updatePlaybackBlocked();
}

function attachWatchIncoming(
  track: MediaStreamTrack,
  stream?: MediaStream,
): void {
  if (track.kind === "audio") {
    if (attachSourceAudio(track, stream, "watch")) return;
    forgetSourceAudioReceiver(track);
    receivedAudioSources.set(track, { role: "watch", source: "voice" });
    if (typeof Audio === "undefined") return;
    if (watchAudio.has(track)) return;
    const el = new Audio();
    el.autoplay = true;
    el.setAttribute("playsinline", "true");
    // One element per track: a MediaStream's multiple audio tracks must not
    // compete for an HTML media element's single selected audio track.
    el.srcObject =
      typeof MediaStream === "undefined"
        ? (stream ?? null)
        : new MediaStream([track]);
    watchAudio.set(track, el);
    track.addEventListener("ended", () => {
      if (watchAudio.get(track) !== el) return;
      watchAudio.delete(track);
      el.pause?.();
      el.srcObject = null;
      blockedPlayback.delete(el);
      updatePlaybackBlocked();
    });
    applyPlayback();
    playAudio(el);
    return;
  }
  const parsed = parseIncomingVideo(track, stream);
  const attached = stream ?? new MediaStream([track]);
  const state = useVoice.getState();
  if (!parsed) {
    // An untagged track cannot identify the selected publisher.
    return;
  }
  if (
    parsed.k !== "l" ||
    (state.watchPublisherId && state.watchPublisherId !== parsed.userId)
  )
    return;
  useVoice.setState({
    watchStream: attached,
    watchPublisherId: parsed.userId,
  });
}

async function applyWatchRemote(
  type: "offer" | "answer",
  sdp: string,
  mine: number,
): Promise<void> {
  const pc = watchCall.pc;
  const current = () => watchCall.generation === mine && watchCall.pc === pc;
  if (!pc || !current()) return;
  if (type === "answer" && watchCall.signalingState() !== "have-local-offer")
    return;
  if (type === "offer") {
    const collision =
      watchCall.makingOffer || watchCall.signalingState() !== "stable";
    if (collision) {
      watchCall.needOffer = true;
      noteRestartRolledBack(watchRecoverySlot, mine);
      try {
        await pc.setLocalDescription({ type: "rollback" });
      } catch {
        // implicit rollback
      }
    }
    if (!current()) return;
    watchCall.sfuOffered = true;
  }
  await pc.setRemoteDescription({ type, sdp });
  if (!current()) return;
  const queued = watchCall.pendingIce;
  watchCall.pendingIce = [];
  for (const candidate of queued) {
    try {
      await pc.addIceCandidate(candidate);
    } catch (error) {
      if (current()) {
        logVoice("warn", "watch-ice", {
          detail: error instanceof Error ? error.message : "addIceCandidate",
          mid: candidate.sdpMid ?? undefined,
        });
        noteDiagnosticEvent({
          kind: "ice-error",
          connection: "watch",
          detail: "addIceCandidate",
        });
      }
    }
    if (!current()) return;
  }
  if (type === "offer") {
    const answer = withTunedSdp(await pc.createAnswer(), pc);
    if (!current()) return;
    await pc.setLocalDescription(answer);
    if (!current()) return;
    if (answer.sdp) {
      logVoice("info", "watch-answer", { bytes: answer.sdp.length });
      watchCall.send({ op: "a", sdp: answer.sdp });
    }
  }
  if (
    watchCall.needOffer ||
    owesIceRestart(watchRecoverySlot, watchCall.generation)
  ) {
    void watchOfferIfStable(watchCall.generation);
  }
}

function onWatchFrame(frame: MediaServerFrame): void {
  const mine = watchCall.generation;
  const fail = (error: unknown) => {
    if (watchCall.generation !== mine) return;
    noteDiagnosticEvent({
      kind: "sdp-error",
      connection: "watch",
      detail: error instanceof Error ? error.message : "sdp",
    });
    stopWatching();
    deps?.onError?.(error);
  };
  if (frame.op === "ok") {
    watchCall.accept();
    return;
  }
  if (frame.op === "err") {
    logVoice("warn", "watch-error", { op: frame.op, code: frame.e });
    if (frame.e === "ice_failed") {
      noteDiagnosticEvent({
        kind: "ice-error",
        connection: "watch",
        detail: "ice_failed",
      });
      restartWatchTransport(mine, { allowReconnect: false });
      return;
    }
    if (frame.e === "unavailable") {
      const state = useVoice.getState();
      if (watchRetry.active && state.watching && state.watchChannelId) {
        stopWatchPeer(true);
        scheduleWatchRebuild(state.watchChannelId);
        return;
      }
      if (watchReported) return;
      watchReported = true;
      stopWatching();
      deps?.onError?.(new Error("Kein freier Sprachplatz."));
      return;
    }
    if (watchReported) return;
    watchReported = true;
    if (frame.e !== "unauthorized") {
      noteDiagnosticEvent({
        kind: "sdp-error",
        connection: "watch",
        detail: frame.e,
      });
    }
    stopWatching();
    deps?.onError?.(
      new Error(
        frame.e === "unauthorized"
          ? errorMessage("unauthenticated")
          : "Der Stream konnte nicht verbunden werden.",
      ),
    );
    return;
  }
  if (frame.op === "a" && frame.sdp) {
    void watchCall
      .enqueue(() => applyWatchRemote("answer", frame.sdp, mine))
      .catch(fail);
    return;
  }
  if (frame.op === "o" && frame.sdp) {
    void watchCall
      .enqueue(() => applyWatchRemote("offer", frame.sdp, mine))
      .catch(fail);
    return;
  }
  if (frame.op === "i" && frame.ice) {
    const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null };
    if (watchCall.pc?.remoteDescription) {
      void watchCall.pc.addIceCandidate(candidate).catch((error) => {
        if (watchCall.generation === mine) {
          logVoice("warn", "watch-ice", {
            detail: error instanceof Error ? error.message : "addIceCandidate",
            mid: candidate.sdpMid ?? undefined,
          });
          noteDiagnosticEvent({
            kind: "ice-error",
            connection: "watch",
            detail: "addIceCandidate",
          });
        }
      });
    } else {
      watchCall.pendingIce.push(candidate);
    }
  }
}

async function watchOfferIfStable(
  mine: number,
  opts?: { initial?: boolean; fromEvent?: boolean; iceRestart?: boolean },
): Promise<void> {
  const epoch = watchEpoch;
  const peer = watchCall;
  const live = () =>
    watchEpoch === epoch &&
    watchCall === peer &&
    watchCall.generation === mine &&
    !!watchCall.pc;
  await peer.enqueue(async () => {
    if (!live()) return;
    if (opts?.initial && watchCall.sfuOffered) return;
    const restart = wantsIceRestart(watchRecoverySlot, mine, opts?.iceRestart);
    const sticky = restart || (!opts?.initial && !opts?.fromEvent);
    if (watchCall.makingOffer || watchCall.signalingState() !== "stable") {
      if (sticky) watchCall.needOffer = true;
      if (restart) markIceRestartPending(watchRecoverySlot, mine);
      return;
    }
    watchCall.makingOffer = true;
    watchCall.needOffer = false;
    try {
      if (watchCall.signalingState() !== "stable") {
        if (sticky) watchCall.needOffer = true;
        if (restart) markIceRestartPending(watchRecoverySlot, mine);
        return;
      }
      if (opts?.initial && watchCall.sfuOffered) return;
      const pc = watchCall.pc;
      if (!pc) return;
      preferOpus(pc);
      preferVp8(pc);
      const offer = withTunedSdp(
        await pc.createOffer(restart ? { iceRestart: true } : undefined),
        pc,
      );
      if (!live()) return;
      if (watchCall.signalingState() !== "stable") {
        if (sticky) watchCall.needOffer = true;
        if (restart) markIceRestartPending(watchRecoverySlot, mine);
        return;
      }
      await pc.setLocalDescription(offer);
      if (!live()) return;
      if (restart) noteRestartOfferCreated(watchRecoverySlot, mine);
      if (!offer.sdp) return;
      watchCall.send({ op: "o", sdp: offer.sdp });
    } catch (error) {
      if (restart && live()) markIceRestartPending(watchRecoverySlot, mine);
      if (live()) {
        noteDiagnosticEvent({
          kind: "sdp-error",
          connection: "watch",
          detail: error instanceof Error ? error.message : "sdp",
        });
        deps?.onError?.(error);
      }
    } finally {
      if (live()) watchCall.makingOffer = false;
    }
  });
}

function scheduleWatchRebuild(channelId: string): void {
  const mine = watchCall.generation;
  watchRetry.schedule(
    () => {
      const state = useVoice.getState();
      if (
        watchCall.generation !== mine ||
        !state.watching ||
        state.watchChannelId !== channelId
      )
        return;
      void startWatchPeer(channelId, true);
    },
    () => {
      stopWatching();
      deps?.onError?.(
        new Error(
          "Der Stream konnte nicht wieder verbunden werden. Bitte erneut ansehen.",
        ),
      );
    },
  );
}

async function startWatchPeer(
  channelId: string,
  recovering = false,
): Promise<void> {
  const mine = watchCall.generation + 1;
  stopWatchPeer(recovering);
  watchCall.generation = mine;
  const fetchTicket = deps?.fetchTicket ?? requestMediaTicket;
  const openMedia = deps?.openMedia ?? openMediaSocket;
  const createPeer = deps?.createPeer ?? defaultCreatePeer;

  let iceServers: IceServer[];
  try {
    const ticket = await fetchTicket(channelId);
    if (watchCall.generation !== mine) return;
    iceServers = ticket.ice_servers ?? [];
    const socket = openMedia(mediaWsUrl(ticket.media_path));
    watchCall.bind(
      socket,
      (frame) => {
        if (watchCall.generation === mine) onWatchFrame(frame);
      },
      () => {
        if (watchCall.generation !== mine) return;
        const state = useVoice.getState();
        if (state.watching && state.watchChannelId === channelId) {
          scheduleWatchRebuild(channelId);
        }
      },
    );
    const publisher = useVoice.getState().watchPublisherId;
    watchCall.send({
      op: "j",
      tk: ticket.ticket,
      v: 2,
      ...(publisher ? { w: publisher } : {}),
    });
  } catch (error) {
    if (watchCall.generation !== mine) return;
    if (recovering && retryableTicketError(error)) {
      stopWatchPeer(true);
      scheduleWatchRebuild(channelId);
      return;
    }
    stopWatching();
    deps?.onError?.(error);
    return;
  }

  if (watchCall.generation !== mine) return;
  const pc = createPeer(iceServers);
  watchCall.pc = pc;
  clearWatchReconnectTimer();
  clearRecovery(watchRecoverySlot);
  attachWatchDiagnostics(mine);
  pc.onnegotiationneeded = () => {
    if (watchCall.generation !== mine) return;
    void watchOfferIfStable(mine, { fromEvent: true });
  };
  pc.oniceconnectionstatechange = () => {
    noteIceState(pc, "watch", mine, () => watchCall.generation);
    onWatchConnectionChange(mine);
  };
  pc.onconnectionstatechange = () => {
    onWatchConnectionChange(mine);
  };
  // Recvonly m-lines so the offer carries ice-ufrag. webrtc-rs rejects
  // an empty offer with "set_remote_description called with no ice-ufrag".
  pc.addTransceiver?.("audio", { direction: "recvonly" });
  pc.addTransceiver?.("video", { direction: "recvonly" });
  preferOpus(pc);
  preferVp8(pc);
  pc.onicecandidate = (event) => {
    if (watchCall.generation !== mine) return;
    if (!event.candidate?.candidate) return;
    watchCall.send({
      op: "i",
      ice: event.candidate.candidate,
      ...(event.candidate.sdpMid ? { mid: event.candidate.sdpMid } : {}),
    });
  };
  pc.ontrack = (event) => {
    if (watchCall.generation !== mine) return;
    attachWatchIncoming(event.track, event.streams[0]);
  };

  if (watchCall.generation !== mine) return;
  await watchOfferIfStable(mine, { initial: true });
}
