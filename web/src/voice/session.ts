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
  requestMediaTicket,
  tuneAudioSdp,
} from "./media.ts";
import { type IceCand, MediaPeer } from "./mediaPeer.ts";
import {
  type MediaSettings,
  audioBitrate,
  cameraConstraints,
  displayConstraints,
  VIDEO_SEND_BUDGET,
  VIDEO_MAX_FPS,
  micConstraints,
  onMediaSettingsChange,
  useMediaSettings,
} from "./settings.ts";

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
  localCamera: MediaStream | null;
  localScreen: MediaStream | null;
  localLive: MediaStream | null;
  watching: boolean;
  watchServerId: string | null;
  watchChannelId: string | null;
  watchStream: MediaStream | null;
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
  localCamera: null,
  localScreen: null,
  localLive: null,
  watching: false,
  watchServerId: null,
  watchChannelId: null,
  watchStream: null,
  remote: {},
  participants: {},
};

export const useVoice = create<VoiceState>(() => ({ ...idle }));

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
  createOffer(): Promise<{ type: string; sdp?: string }>;
  createAnswer(): Promise<{ type: string; sdp?: string }>;
  setLocalDescription(desc: { type: string; sdp?: string }): Promise<void>;
  setRemoteDescription(desc: { type: string; sdp?: string }): Promise<void>;
  getTransceivers?(): {
    sender?: { track?: { kind: string } | null };
    receiver?: { track?: { kind: string } | null };
    setCodecPreferences?(codecs: { mimeType: string }[]): void;
  }[];
  addIceCandidate(candidate: {
    candidate: string;
    sdpMid: string | null;
  }): Promise<void>;
  close(): void;
  remoteDescription?: { type: string } | null;
  signalingState?: string;
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

type MicGainInsert = {
  stream: MediaStream;
  setGain: (gain: number) => void;
  dispose: () => void;
};

let deps: VoiceDeps | null = null;
/** Seat media peer. `joinVoice` / `leaveVoice` hold this object. */
let seat = new MediaPeer();
let localStream: MediaStream | null = null;
let rawMicStream: MediaStream | null = null;
let activeMicGain: MicGainInsert | null = null;
let cameraStream: MediaStream | null = null;
let screenStream: MediaStream | null = null;
let liveStream: MediaStream | null = null;
let remoteAudio: HTMLAudioElement | null = null;
let watchAudio: HTMLAudioElement | null = null;
let remoteMix: MediaStream | null = null;
let bound = false;
let micEpoch = 0;
/** One stream toast per join. Further failures stay in the console. */
let streamReported = false;
let watchReported = false;
let cameraEpoch = 0;
let screenEpoch = 0;
let liveEpoch = 0;
let awaitingJoin: { serverId: string; channelId: string } | null = null;
let awaitingLive: { serverId: string; channelId: string } | null = null;
let audioCommitChain: Promise<void> = Promise.resolve();
let cameraCommitChain: Promise<void> = Promise.resolve();
/** Senders that belonged to the last completed negotiation. */
let settledSenders: RtpSender[] = [];
/** Video kinds announced for the offer that is still unanswered. */
let openPublish: Array<"v" | "s" | "l"> = [];
/** Cleanup of a rejected publish must not enqueue a replacement offer. */
let discardingPublish = false;
/** Gateway reconnect left the media peer up; re-announce after our join echo. */
let republishOnJoin = false;
/** Kinds this media attempt has already announced on the gateway. */
const announced = new Set<TrackKind>();
const pendingMicRaw = new Set<MediaStream>();
const pendingCameraStreams = new Set<MediaStream>();

/** Watch media peer. `watchLive` / `stopWatching` hold this object. */
let watchCall = new MediaPeer();

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
  void remoteAudio.play()?.catch(() => undefined);
  applyLocalAudio();
  applyPlayback();
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

function hintTrack(track: MediaStreamTrack, hint: "speech" | "detail"): void {
  try {
    (track as MediaStreamTrack & { contentHint?: string }).contentHint = hint;
  } catch {
    // contentHint is best-effort
  }
}

function withTunedSdp(desc: { type: string; sdp?: string }): {
  type: string;
  sdp?: string;
} {
  if (!desc.sdp) return desc;
  return { type: desc.type, sdp: tuneAudioSdp(desc.sdp) };
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
): { userId: string; k: "v" | "s" | "l" } | null {
  const match = /^(.*):(v|s|l)(?:[-:].*)?$/.exec(id);
  const userId = match?.[1];
  const k = match?.[2];
  if (userId && (k === "v" || k === "s" || k === "l")) {
    return { userId, k };
  }
  return null;
}

function onSig(event: SigEvent): void {
  const state = useVoice.getState();
  const watchingHere =
    state.watching &&
    event.s === state.watchServerId &&
    event.c === state.watchChannelId;
  const inRoom =
    state.status === "joined" &&
    event.s === state.serverId &&
    event.c === state.channelId;
  if (
    watchingHere &&
    ((event.t === "u" && event.k === "l") || event.t === "l")
  ) {
    const liveUser = event.c
      ? liveOf(useVoiceRoster.getState().live, event.s, event.c)
      : null;
    if (
      event.t === "u" ||
      (event.t === "l" && event.u && liveUser === event.u)
    ) {
      stopWatching();
    }
  }
  if (!inRoom) {
    return;
  }
  const userId = event.u;
  if (!userId) return;
  switch (event.t) {
    case "j":
      if (
        userId === currentUserId() &&
        awaitingJoin &&
        event.c === awaitingJoin.channelId
      ) {
        awaitingJoin = null;
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
        awaitingLive = null;
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
      if (event.k === "v" || event.k === "s" || event.k === "l") {
        if (event.t === "u") {
          // Gateway `t:"l"` keeps a track that is still arriving. `t:"u"`
          // means that camera, screen, or Go Live actually ended.
          forgetReceived(userId, event.k);
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
        watchStream: state.watchStream,
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
  // Rebuild only when the media transport has actually closed or failed.
  if (keepMedia) return;
  void startPeer(state.serverId, state.channelId);
}

function stopTracks(stream: MediaStream | null): void {
  stream?.getTracks().forEach((track) => track.stop());
}

function stopPeer(): void {
  streamReported = false;
  seat.close();
  micEpoch += 1;
  cameraEpoch += 1;
  screenEpoch += 1;
  liveEpoch += 1;
  audioCommitChain = Promise.resolve();
  cameraCommitChain = Promise.resolve();
  settledSenders = [];
  openPublish = [];
  discardingPublish = false;
  announced.clear();
  for (const stream of pendingMicRaw) stopTracks(stream);
  pendingMicRaw.clear();
  for (const stream of pendingCameraStreams) stopTracks(stream);
  pendingCameraStreams.clear();
  stopTracks(localStream);
  stopTracks(rawMicStream);
  stopTracks(cameraStream);
  stopTracks(screenStream);
  stopTracks(liveStream);
  disposeMicGain();
  localStream = null;
  rawMicStream = null;
  cameraStream = null;
  screenStream = null;
  liveStream = null;
  remoteMix = null;
  clearReceived();
  if (remoteAudio) {
    remoteAudio.srcObject = null;
  }
  useVoice.setState({
    localCamera: null,
    localScreen: null,
    localLive: null,
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
    watchStream: state.watchStream,
  });
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
  for (const el of [remoteAudio, watchAudio]) {
    if (!el) continue;
    el.muted = deafened;
    el.volume = volume;
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
      encoding.maxBitrate = bitrate;
    }
    try {
      await sender.setParameters?.(params);
    } catch {
      // Chromium rejects setParameters before the first description.
    }
  }
}

async function applyVideoLimits(pc: PeerConnection): Promise<void> {
  const senders = (pc.getSenders?.() ?? []).filter(
    (s) => s.track?.kind === "video",
  );
  const perSender = Math.floor(VIDEO_SEND_BUDGET / Math.max(1, senders.length));
  for (const sender of senders) {
    const params = sender.getParameters?.();
    if (!params?.encodings.length) continue;
    for (const encoding of params.encodings) {
      encoding.maxBitrate = Math.floor(perSender / params.encodings.length);
      encoding.maxFramerate = VIDEO_MAX_FPS;
    }
    try {
      await sender.setParameters?.(params);
    } catch {
      // Some browsers require negotiation first; retry after SDP completes.
    }
  }
}

function audioContextCtor(): { new (): AudioContext } | undefined {
  return (
    globalThis as unknown as {
      AudioContext?: { new (): AudioContext };
    }
  ).AudioContext;
}

function disposeMicGain(): void {
  const insert = activeMicGain;
  activeMicGain = null;
  insert?.dispose();
}

/** Locally owned gain graph — does not touch the active session insert. */
function createMicGainInsert(
  raw: MediaStream,
  gain: number,
): MicGainInsert | null {
  const Ctx = audioContextCtor();
  if (!Ctx) return null;
  try {
    const ctx = new Ctx();
    const src = ctx.createMediaStreamSource(raw);
    const node = ctx.createGain();
    node.gain.value = gain;
    const dest = ctx.createMediaStreamDestination();
    src.connect(node);
    node.connect(dest);
    void ctx.resume();
    let disposed = false;
    return {
      stream: dest.stream,
      setGain(next) {
        if (disposed) return;
        node.gain.value = next;
        if (ctx.state === "suspended") void ctx.resume();
      },
      dispose() {
        if (disposed) return;
        disposed = true;
        try {
          node.disconnect();
        } catch {
          // already disconnected
        }
        if (ctx.state !== "closed") {
          void ctx.close();
        }
      },
    };
  } catch {
    return null;
  }
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
  send.getAudioTracks().forEach((track) => hintTrack(track, "speech"));
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

function buildMicCandidate(raw: MediaStream): {
  send: MediaStream;
  insert: MicGainInsert | null;
} {
  const gain = useMediaSettings.getState().inputGain;
  if (gain === 1) {
    return { send: raw, insert: null };
  }
  const insert = createMicGainInsert(raw, gain);
  if (!insert) return { send: raw, insert: null };
  disableAudio(insert.stream);
  return { send: insert.stream, insert };
}

async function refreshMic(): Promise<void> {
  const pc = seat.pc;
  if (!pc || useVoice.getState().status !== "joined") return;
  const session = seat.generation;
  const request = ++micEpoch;
  const getUserMedia = deps?.getUserMedia ?? defaultGetUserMedia;
  let raw: MediaStream;
  try {
    raw = await getUserMedia({ audio: micConstraints(), video: false });
  } catch {
    return;
  }
  if (!micCurrent(pc, session, request)) {
    stopTracks(raw);
    return;
  }
  disableAudio(raw);
  pendingMicRaw.add(raw);
  const { send, insert } = buildMicCandidate(raw);
  await enqueueAudioCommit(async () => {
    if (!micCurrent(pc, session, request)) {
      insert?.dispose();
      stopTracks(raw);
      pendingMicRaw.delete(raw);
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
    if (!micCurrent(pc, session, request)) {
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
      insert?.dispose();
      stopTracks(raw);
      pendingMicRaw.delete(raw);
      return;
    }
    const oldRaw = rawMicStream;
    const oldSend = localStream;
    const oldInsert = activeMicGain;
    rawMicStream = raw;
    localStream = send;
    activeMicGain = insert;
    pendingMicRaw.delete(raw);
    applyLocalAudio();
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
  const getUserMedia = deps?.getUserMedia ?? defaultGetUserMedia;
  let stream: MediaStream;
  try {
    stream = await getUserMedia({
      audio: false,
      video: cameraConstraints(),
    });
  } catch {
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
      if (seat.generation === session && seat.pc === pc && sender?.replaceTrack) {
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
    if (!rawMicStream || useVoice.getState().status !== "joined") return;
    const pc = seat.pc;
    if (!pc) return;
    const session = seat.generation;
    const request = micEpoch;
    const gain = useMediaSettings.getState().inputGain;
    if (activeMicGain) {
      activeMicGain.setGain(gain);
      if (gain !== 1) {
        return;
      }
      const dest = localStream;
      const raw = rawMicStream;
      const sender = audioSender();
      const previous = sender?.track ?? null;
      let outcome: "replaced" | "added" | "none";
      try {
        outcome = await commitMicSend(pc, raw, session);
      } catch (error) {
        if (micCurrent(pc, session, request)) deps?.onError?.(error);
        return;
      }
      if (!micCurrent(pc, session, request)) {
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
        return;
      }
      localStream = raw;
      if (dest && dest !== raw) stopTracks(dest);
      disposeMicGain();
      applyLocalAudio();
      return;
    }
    if (gain === 1) return;
    const insert = createMicGainInsert(rawMicStream, gain);
    if (!insert) return;
    disableAudio(insert.stream);
    const sender = audioSender();
    const previous = sender?.track ?? null;
    let outcome: "replaced" | "added" | "none";
    try {
      outcome = await commitMicSend(pc, insert.stream, session);
    } catch (error) {
      insert.dispose();
      if (micCurrent(pc, session, request)) deps?.onError?.(error);
      return;
    }
    if (!micCurrent(pc, session, request)) {
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
      insert.dispose();
      return;
    }
    localStream = insert.stream;
    activeMicGain = insert;
    applyLocalAudio();
  });
}

function queueInputGain(): void {
  void applyInputGain();
}

function audioSender(): RtpSender | undefined {
  return seat.pc?.getSenders?.().find((sender) => sender.track?.kind === "audio");
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
    prev.autoGainControl !== next.autoGainControl;
  const camChanged = prev.videoInputId !== next.videoInputId;
  const qualityChanged = prev.quality !== next.quality;
  if (joined && recaptureMic) void refreshMic();
  else if (joined && prev.inputGain !== next.inputGain) queueInputGain();
  if (joined && camChanged && useVoice.getState().camera) void refreshCamera();
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
    sendPub("a", true);
  }
  if (state.camera && cameraStream) sendPub("v", true);
  if (state.sharing && screenStream) sendPub("s", true);
  if (!state.live) return;
  const self = currentUserId();
  if (self) {
    applyLiveStart(state.serverId, state.channelId, self);
    awaitingLive = { serverId: state.serverId, channelId: state.channelId };
  }
  sendPub("l", true);
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
        // The failed offer's follow-up must wait for the next user publish.
        seat.needOffer = false;
        for (const kind of kinds) {
          logVoice("warn", "unpublish", { track: kind });
          seat.send({ op: "u", k: kind });
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
    if (seat.needOffer) void offerIfStable(mine);
  });
}

/** Remove senders added after the last successful answer. The mic stays. */
function dropUnsettledPublish(): Array<"v" | "s" | "l"> {
  const pc = seat.pc;
  const kinds = openPublish.slice();
  openPublish = [];
  if (pc?.getSenders && pc.removeTrack) {
    for (const sender of pc.getSenders()) {
      if (settledSenders.includes(sender)) continue;
      pc.removeTrack(sender);
    }
  }
  for (const kind of kinds) releaseDiscardedCapture(kind);
  return kinds;
}

/**
 * Stop a publish that never negotiated: capture, local preview, and flags.
 * Does not touch the mic or a stream that already completed an answer, and
 * does not ask for a new offer.
 */
function releaseDiscardedCapture(kind: "v" | "s" | "l"): void {
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
  stopTracks(stream);
  const self = currentUserId();
  if (self) setPub(self, kind, false);
  sendPub(kind, false);
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
      try {
        await pc.setLocalDescription({ type: "rollback" });
      } catch {
        // Browsers may perform implicit rollback in setRemoteDescription.
      }
      if (!current()) return;
    }
    seat.sfuOffered = true;
  }
  await pc.setRemoteDescription({ type, sdp: tuneAudioSdp(sdp) });
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
  await applyVideoLimits(pc);
  if (!current()) return;
  seat.negotiated = true;
  if (type === "answer") {
    settledSenders = [...(pc.getSenders?.() ?? [])];
    openPublish = [];
  }
  if (seat.needOffer) void offerIfStable(mine);
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
      }
    }
    return;
  }
  seat.pendingIce.push(candidate);
}

function onMediaFrame(frame: MediaServerFrame): void {
  const mine = seat.generation;
  if (frame.op === "ok") {
    seat.accept();
    return;
  }
  if (frame.op === "err") {
    logVoice("warn", "media-error", { op: frame.op, code: frame.e });
    if (frame.e === "ice_failed") return;
    if (frame.e === "unavailable") {
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
      rollbackSeat(new ApiError("bad_request", 0, errorMessage("bad_request")));
      return;
    }
    reportStreamOnce(frame.e);
    void settleFailedNegotiation(mine);
    return;
  }
  if ((frame.op === "a" || frame.op === "o") && frame.sdp) {
    const type = frame.op === "a" ? "answer" : "offer";
    logVoice("info", type === "offer" ? "recv-offer" : "recv-answer", {
      bytes: frame.sdp.length,
    });
    void seat.enqueue(() => applyRemoteDescription(type, frame.sdp, mine)).catch(
      (error) => recoverNegotiation(error, mine),
    );
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
  ensureBound();
  const userId = currentUserId();
  if (!userId) return;
  streamReported = false;
  const prev = useVoice.getState();
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
  if (
    useVoice.getState().watchServerId === input.serverId &&
    useVoice.getState().watchChannelId === input.channelId
  ) {
    stopWatching();
  }
  const self = userId;
  useVoice.setState({
    status: "joined",
    serverId: input.serverId,
    channelId: input.channelId,
    channelName: input.channelName,
    muted: false,
    deafened: false,
    camera: false,
    sharing: false,
    live: false,
    localCamera: null,
    localScreen: null,
    localLive: null,
    remote: {},
    participants: { [self]: { pubs: [] } },
  });
  applyVoiceJoin(input.serverId, self, input.channelId, {
    muted: false,
    deafened: false,
  });
  awaitingJoin = { serverId: input.serverId, channelId: input.channelId };
  deps?.gateway.send({
    op: "sig",
    t: "j",
    s: input.serverId,
    c: input.channelId,
  });
  void startPeer(input.serverId, input.channelId);
}

export function leaveVoice(): void {
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
    watchStream: state.watchStream,
  });
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
  useVoice.setState({ muted, deafened });
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
 * unmutes. The member list is updated before the server round-trip.
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
  const muted = deafened;
  useVoice.setState({ muted, deafened });
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
  awaitingLive = { serverId: state.serverId, channelId: state.channelId };
  void startLocalVideo("l");
}

/**
 * Subscribe to a live track without publishing. No mic permission prompt.
 */
export function watchLive(input: {
  serverId: string;
  channelId: string;
  channelName: string;
}): void {
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
  useVoice.setState({
    watching: true,
    watchServerId: input.serverId,
    watchChannelId: input.channelId,
    watchStream: null,
  });
  void startWatchPeer(input.channelId);
}

export function stopWatching(): void {
  stopWatchPeer();
  useVoice.setState({
    watching: false,
    watchServerId: null,
    watchChannelId: null,
    watchStream: null,
  });
}

export function resetVoiceForTests(): void {
  streamReported = false;
  watchReported = false;
  awaitingJoin = null;
  awaitingLive = null;
  republishOnJoin = false;
  seat.pendingIce = [];
  audioCommitChain = Promise.resolve();
  cameraCommitChain = Promise.resolve();
  stopWatchPeer();
  stopPeer();
  seat = new MediaPeer();
  watchCall = new MediaPeer();
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
  for (const track of stream.getTracks()) {
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
  const getMedia =
    kind === "v"
      ? (deps?.getUserMedia ?? defaultGetUserMedia)
      : (deps?.getDisplayMedia ?? defaultGetDisplayMedia);
  // Video only for camera / screen / live. Display audio mixed into the
  // voice m-line (and tagged as a second "a" pub) was echo-y on deploy and
  // added a video-sized SDP the 12 KiB cap then rejected.
  const constraints: MediaStreamConstraints =
    kind === "v"
      ? { audio: false, video: cameraConstraints() }
      : { audio: false, video: displayConstraints() };
  let stream: MediaStream;
  try {
    stream = await getMedia(constraints);
  } catch {
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
  if (seat.generation !== mine || videoEpoch(kind) !== epoch) {
    stopTracks(stream);
    return;
  }
  const self = currentUserId();
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
  if (self) setPub(self, kind, true);
  // Yield so the local tile paints before addTrack / offer.
  await Promise.resolve();
  if (seat.generation !== mine || videoEpoch(kind) !== epoch) {
    return;
  }
  await publishLocal(kind, stream);
}

function stopLocalVideo(kind: "v" | "s" | "l"): void {
  const state = useVoice.getState();
  if (kind === "v" && !state.camera && !state.localCamera) return;
  if (kind === "s" && !state.sharing && !state.localScreen) return;
  if (kind === "l" && !state.live && !state.localLive) return;
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
    if (sender) seat.pc?.removeTrack?.(sender);
    track.stop();
  }
  if (seat.pc) void applyVideoLimits(seat.pc);
  seat.needOffer = true;
  void offerIfStable(seat.generation);
}

async function publishLocal(
  kind: "v" | "s" | "l",
  stream: MediaStream,
): Promise<void> {
  if (!seat.pc) return;
  const tracks = stream.getVideoTracks();
  if (tracks.length === 0) return;
  logVoice("info", "publish", { track: kind });
  openPublish.push(kind);
  seat.send({ op: "p", k: kind });
  for (const track of tracks) {
    seat.pc.addTrack?.(track, stream);
  }
  const pc = seat.pc;
  void applyVideoLimits(pc);
  const self = currentUserId();
  if (self) setPub(self, kind, true);
  sendPub(kind, true);
  seat.needOffer = true;
  await offerIfStable(seat.generation);
}

function parseIncomingVideo(
  track: MediaStreamTrack,
  stream?: MediaStream,
): { userId: string; k: "v" | "s" | "l" } | null {
  return parseRemoteStreamId(stream?.id ?? "") ?? parseRemoteStreamId(track.id);
}

function attachIncoming(track: MediaStreamTrack, stream?: MediaStream): void {
  if (track.kind === "audio") {
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
  opts?: { initial?: boolean; fromEvent?: boolean },
): Promise<void> {
  if (discardingPublish) return;
  await seat.enqueue(async () => {
    if (discardingPublish) return;
    if (seat.generation !== mine || !seat.pc) return;
    if (opts?.initial && seat.sfuOffered) return;
    if (seat.makingOffer || seat.signalingState() !== "stable") {
      // Sticky only for explicit publish/unpublish. negotiationneeded
      // fires again once we are stable (W3C perfect negotiation).
      if (!opts?.initial && !opts?.fromEvent) seat.needOffer = true;
      return;
    }
    seat.makingOffer = true;
    seat.needOffer = false;
    try {
      if (seat.signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) seat.needOffer = true;
        return;
      }
      if (opts?.initial && seat.sfuOffered) return;
      preferOpus(seat.pc);
      preferVp8(seat.pc);
      const pc = seat.pc;
      const offer = withTunedSdp(await pc.createOffer());
      if (seat.generation !== mine) return;
      if (seat.signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) seat.needOffer = true;
        return;
      }
      await pc.setLocalDescription(offer);
      if (seat.generation !== mine || !offer.sdp) return;
      logVoice("info", "send-offer", { bytes: offer.sdp.length });
      seat.send({ op: "o", sdp: offer.sdp });
    } catch (error) {
      if (seat.generation === mine) recoverNegotiation(error, mine);
    } finally {
      if (seat.generation === mine) seat.makingOffer = false;
    }
  });
}

async function startPeer(serverId: string, channelId: string): Promise<void> {
  const mine = seat.generation + 1;
  const resumeCamera = useVoice.getState().camera;
  const resumeShare = useVoice.getState().sharing;
  const resumeLive = useVoice.getState().live;
  stopPeer();
  seat.generation = mine;
  if (resumeCamera) useVoice.setState({ camera: true });
  if (resumeShare) useVoice.setState({ sharing: true });
  if (resumeLive) useVoice.setState({ live: true });
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
        void startPeer(state.serverId, state.channelId);
      },
    );
    seat.send({ op: "j", tk: ticket.ticket });
  } catch (error) {
    if (seat.generation !== mine) return;
    rollbackSeat(error);
    return;
  }

  const pc = createPeer(iceServers);
  seat.pc = pc;

  pc.onnegotiationneeded = () => {
    if (seat.generation !== mine) return;
    void offerIfStable(mine, { fromEvent: true });
  };
  pc.onicecandidate = (event) => {
    if (seat.generation !== mine) return;
    if (!event.candidate) return;
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

  const micRequest = ++micEpoch;
  try {
    const stream = await getUserMedia({
      audio: micConstraints(),
      video: false,
    });
    if (seat.generation !== mine) {
      stopTracks(stream);
      return;
    }
    if (micEpoch !== micRequest) {
      stopTracks(stream);
      // A newer refreshMic owns capture; continue without this stream.
    } else {
      disableAudio(stream);
      pendingMicRaw.add(stream);
      const { send, insert } = buildMicCandidate(stream);
      await enqueueAudioCommit(async () => {
        if (seat.generation !== mine || micEpoch !== micRequest || seat.pc !== pc) {
          insert?.dispose();
          stopTracks(stream);
          pendingMicRaw.delete(stream);
          return;
        }
        rawMicStream = stream;
        localStream = send;
        activeMicGain = insert;
        pendingMicRaw.delete(stream);
        send.getAudioTracks().forEach((track) => hintTrack(track, "speech"));
        applyLocalAudio();
        for (const track of send.getTracks()) {
          pc.addTrack?.(track, send);
        }
        try {
          await applySendBitrate();
        } catch {
          // bitrate is best-effort
        }
        preferOpus(pc);
        preferVp8(pc);
        const self = currentUserId();
        if (self && useVoice.getState().channelId === channelId) {
          setPub(self, "a", true);
        }
        announced.add("a");
        deps?.gateway.send({
          op: "sig",
          t: "p",
          s: serverId,
          c: channelId,
          k: "a",
        });
      });
    }
  } catch {
    // No mic — still send an offer so ICE can run.
  }

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
  if ((pending.camera || resumeCamera) && !useVoice.getState().localCamera) {
    void startLocalVideo("v");
  }
  if ((pending.sharing || resumeShare) && !useVoice.getState().localScreen) {
    void startLocalVideo("s");
  }
  if ((pending.live || resumeLive) && !useVoice.getState().localLive) {
    void startLocalVideo("l");
  }
  if (seat.generation !== mine) return;
  await offerIfStable(mine, { initial: true });
}

function stopWatchPeer(): void {
  watchReported = false;
  watchCall.close();
  if (watchAudio) {
    watchAudio.srcObject = null;
  }
}

function attachWatchIncoming(
  track: MediaStreamTrack,
  stream?: MediaStream,
): void {
  if (track.kind === "audio") {
    if (typeof Audio === "undefined") return;
    if (!watchAudio) {
      watchAudio = new Audio();
      watchAudio.autoplay = true;
      watchAudio.setAttribute("playsinline", "true");
    }
    const mix = stream ?? new MediaStream([track]);
    watchAudio.srcObject = mix;
    void watchAudio.play()?.catch(() => undefined);
    applyPlayback();
    return;
  }
  const parsed = parseIncomingVideo(track, stream);
  const attached = stream ?? new MediaStream([track]);
  const state = useVoice.getState();
  if (!parsed) {
    // First untagged video can fill the tile. Do not replace a parsed live.
    if (!state.watchStream) {
      useVoice.setState({ watchStream: attached });
    }
    return;
  }
  const current = state.remote[parsed.userId] ?? {};
  useVoice.setState({
    watchStream: parsed.k === "l" ? attached : (state.watchStream ?? attached),
    remote: {
      ...state.remote,
      [parsed.userId]: { ...current, [parsed.k]: attached },
    },
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
  if (type === "answer" && watchCall.signalingState() !== "have-local-offer") return;
  if (type === "offer") {
    const collision = watchCall.makingOffer || watchCall.signalingState() !== "stable";
    if (collision) {
      watchCall.needOffer = true;
      try {
        await pc.setLocalDescription({ type: "rollback" });
      } catch {
        // implicit rollback
      }
    }
    if (!current()) return;
    watchCall.sfuOffered = true;
  }
  await pc.setRemoteDescription({ type, sdp: tuneAudioSdp(sdp) });
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
      logVoice("info", "watch-answer", { bytes: answer.sdp.length });
      watchCall.send({ op: "a", sdp: answer.sdp });
    }
  }
  if (watchCall.needOffer) {
    void watchOfferIfStable(watchCall.generation);
  }
}

function onWatchFrame(frame: MediaServerFrame): void {
  const mine = watchCall.generation;
  const fail = (error: unknown) => {
    if (watchCall.generation !== mine) return;
    stopWatching();
    deps?.onError?.(error);
  };
  if (frame.op === "ok") {
    watchCall.accept();
    return;
  }
  if (frame.op === "err") {
    logVoice("warn", "watch-error", { op: frame.op, code: frame.e });
    if (frame.e === "ice_failed") return;
    if (frame.e === "unavailable") {
      if (watchReported) return;
      watchReported = true;
      stopWatching();
      deps?.onError?.(new Error("Kein freier Sprachplatz."));
      return;
    }
    if (watchReported) return;
    watchReported = true;
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
    void watchCall.enqueue(() =>
      applyWatchRemote("answer", frame.sdp, mine),
    ).catch(fail);
    return;
  }
  if (frame.op === "o" && frame.sdp) {
    void watchCall.enqueue(() =>
      applyWatchRemote("offer", frame.sdp, mine),
    ).catch(fail);
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
        }
      });
    } else {
      watchCall.pendingIce.push(candidate);
    }
  }
}

async function watchOfferIfStable(
  mine: number,
  opts?: { initial?: boolean; fromEvent?: boolean },
): Promise<void> {
  await watchCall.enqueue(async () => {
    if (watchCall.generation !== mine || !watchCall.pc) return;
    if (opts?.initial && watchCall.sfuOffered) return;
    if (watchCall.makingOffer || watchCall.signalingState() !== "stable") {
      if (!opts?.initial && !opts?.fromEvent) watchCall.needOffer = true;
      return;
    }
    watchCall.makingOffer = true;
    watchCall.needOffer = false;
    try {
      if (watchCall.signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) watchCall.needOffer = true;
        return;
      }
      if (opts?.initial && watchCall.sfuOffered) return;
      preferOpus(watchCall.pc);
      preferVp8(watchCall.pc);
      const pc = watchCall.pc;
      const offer = withTunedSdp(await pc.createOffer());
      if (watchCall.generation !== mine) return;
      if (watchCall.signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) watchCall.needOffer = true;
        return;
      }
      await pc.setLocalDescription(offer);
      if (watchCall.generation !== mine || !offer.sdp) return;
      watchCall.send({ op: "o", sdp: offer.sdp });
    } catch (error) {
      if (watchCall.generation === mine) {
        deps?.onError?.(error);
      }
    } finally {
      if (watchCall.generation === mine) watchCall.makingOffer = false;
    }
  });
}

async function startWatchPeer(channelId: string): Promise<void> {
  const mine = watchCall.generation + 1;
  stopWatchPeer();
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
          void startWatchPeer(channelId);
        }
      },
    );
    watchCall.send({ op: "j", tk: ticket.ticket });
  } catch (error) {
    if (watchCall.generation !== mine) return;
    stopWatching();
    deps?.onError?.(error);
    return;
  }

  const pc = createPeer(iceServers);
  watchCall.pc = pc;
  pc.onnegotiationneeded = () => {
    if (watchCall.generation !== mine) return;
    void watchOfferIfStable(mine, { fromEvent: true });
  };
  // Recvonly m-lines so the offer carries ice-ufrag. webrtc-rs rejects
  // an empty offer with "set_remote_description called with no ice-ufrag".
  pc.addTransceiver?.("audio", { direction: "recvonly" });
  pc.addTransceiver?.("video", { direction: "recvonly" });
  preferOpus(pc);
  preferVp8(pc);
  pc.onicecandidate = (event) => {
    if (watchCall.generation !== mine) return;
    if (!event.candidate) return;
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
