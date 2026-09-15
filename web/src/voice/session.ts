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
  type MediaSocket,
  type OpenMedia,
  mediaWsUrl,
  openMediaSocket,
  requestMediaTicket,
  tuneAudioSdp,
} from "./media.ts";

const MIC_AUDIO: MediaTrackConstraints = {
  echoCancellation: true,
  noiseSuppression: true,
  autoGainControl: true,
  channelCount: 1,
};

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
  watchChannelId: null,
  watchStream: null,
  remote: {},
  participants: {},
};

export const useVoice = create<VoiceState>(() => ({ ...idle }));

export type RtpSender = { track: MediaStreamTrack | null };

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

type IceCand = { candidate: string; sdpMid: string | null };

let deps: VoiceDeps | null = null;
let peer: PeerConnection | null = null;
let media: MediaSocket | null = null;
let unbindMedia: (() => void) | null = null;
let localStream: MediaStream | null = null;
let cameraStream: MediaStream | null = null;
let screenStream: MediaStream | null = null;
let liveStream: MediaStream | null = null;
let remoteAudio: HTMLAudioElement | null = null;
let watchAudio: HTMLAudioElement | null = null;
let remoteMix: MediaStream | null = null;
let bound = false;
let generation = 0;
let cameraEpoch = 0;
let screenEpoch = 0;
let liveEpoch = 0;
let pendingIce: IceCand[] = [];
let awaitingJoin: { serverId: string; channelId: string } | null = null;
let awaitingLive: { serverId: string; channelId: string } | null = null;
let sdpChain: Promise<void> = Promise.resolve();
let makingOffer = false;
let sfuOffered = false;
let needOffer = false;

let watchPeer: PeerConnection | null = null;
let watchMedia: MediaSocket | null = null;
let unbindWatch: (() => void) | null = null;
let watchGeneration = 0;
let watchPendingIce: IceCand[] = [];
let watchSdpChain: Promise<void> = Promise.resolve();
let watchMakingOffer = false;
let watchSfuOffered = false;
let watchNeedOffer = false;

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
  const watchingHere = state.watching && event.c === state.watchChannelId;
  const inRoom = state.status === "joined" && event.c === state.channelId;
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
        dropRemote(userId);
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
      if (
        event.t === "u" &&
        userId !== currentUserId() &&
        (event.k === "v" || event.k === "s" || event.k === "l")
      ) {
        dropRemote(userId, event.k);
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
    err.c === pending.channelId &&
    (err.e === "forbidden" || err.e === "not_found" || err.e === "bad_request")
  ) {
    const state = useVoice.getState();
    if (state.channelId === pending.channelId) {
      awaitingJoin = null;
      awaitingLive = null;
      stopPeer();
      useVoice.setState({ ...idle });
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
  useVoice.setState({
    participants: self
      ? { [self]: state.participants[self] ?? { pubs: [] } }
      : {},
    remote: {},
  });
  occupySelf(state);
  awaitingJoin = { serverId: state.serverId, channelId: state.channelId };
  deps?.gateway.send({
    op: "sig",
    t: "j",
    s: state.serverId,
    c: state.channelId,
  });
  void startPeer(state.serverId, state.channelId);
}

function stopTracks(stream: MediaStream | null): void {
  stream?.getTracks().forEach((track) => track.stop());
}

function stopPeer(): void {
  generation += 1;
  cameraEpoch += 1;
  screenEpoch += 1;
  liveEpoch += 1;
  pendingIce = [];
  sdpChain = Promise.resolve();
  makingOffer = false;
  sfuOffered = false;
  needOffer = false;
  unbindMedia?.();
  unbindMedia = null;
  media?.close();
  media = null;
  peer?.close();
  peer = null;
  stopTracks(localStream);
  stopTracks(cameraStream);
  stopTracks(screenStream);
  stopTracks(liveStream);
  localStream = null;
  cameraStream = null;
  screenStream = null;
  liveStream = null;
  remoteMix = null;
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

function enqueueSdp(job: () => Promise<void>): Promise<void> {
  const run = sdpChain.then(job, job);
  sdpChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function rollbackSeat(error?: unknown): void {
  const state = useVoice.getState();
  const serverId = state.serverId;
  const channelId = state.channelId;
  awaitingJoin = null;
  awaitingLive = null;
  if (state.live && serverId && channelId) {
    applyLiveEnd(serverId, channelId, currentUserId() ?? undefined);
  }
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...idle,
    watching: state.watching,
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
  if (remoteAudio) {
    remoteAudio.muted = state.deafened;
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
  deps?.gateway.send({
    op: "sig",
    t: on ? "p" : "u",
    s: state.serverId,
    c: state.channelId,
    k: kind,
  });
}

function signalingState(): string {
  return (
    peer?.signalingState ??
    (peer?.remoteDescription ? "have-remote-offer" : "stable")
  );
}

async function applyRemoteDescription(
  type: "offer" | "answer",
  sdp: string,
): Promise<void> {
  if (!peer) return;
  if (type === "offer") {
    const collision = makingOffer || signalingState() !== "stable";
    if (collision) {
      // Polite peer: drop our in-flight offer so a late-joiner can take
      // the SFU's video renegotiation instead of throwing InvalidStateError.
      // Rollback undoes the local description, not addTrack — a follow-up
      // offer is required so a new m-line (camera) actually gets negotiated.
      needOffer = true;
      try {
        await peer.setLocalDescription({ type: "rollback" });
      } catch {
        // Chromium may implicit-rollback inside setRemoteDescription.
      }
    }
    sfuOffered = true;
  }
  await peer.setRemoteDescription({ type, sdp: tuneAudioSdp(sdp) });
  const queued = pendingIce;
  pendingIce = [];
  for (const candidate of queued) {
    await peer.addIceCandidate(candidate);
  }
  if (type === "offer") {
    const answer = withTunedSdp(await peer.createAnswer());
    await peer.setLocalDescription(answer);
    if (answer.sdp) {
      media?.send({ op: "a", sdp: answer.sdp });
    }
  }
  if (needOffer) {
    void offerIfStable(generation);
  }
}

async function applyRemoteIce(candidate: IceCand): Promise<void> {
  if (peer?.remoteDescription) {
    await peer.addIceCandidate(candidate);
    return;
  }
  pendingIce.push(candidate);
}

function onMediaFrame(frame: MediaServerFrame): void {
  if (frame.op === "err") {
    const code: ApiErrorCode =
      frame.e === "unauthorized" ? "unauthenticated" : "bad_request";
    rollbackSeat(new ApiError(code, 0, errorMessage(code)));
    return;
  }
  if (frame.op === "a" && frame.sdp) {
    void enqueueSdp(() => applyRemoteDescription("answer", frame.sdp));
    return;
  }
  if (frame.op === "o" && frame.sdp) {
    void enqueueSdp(() => applyRemoteDescription("offer", frame.sdp));
    return;
  }
  if (frame.op === "i" && frame.ice) {
    void applyRemoteIce({
      candidate: frame.ice,
      sdpMid: frame.mid ?? null,
    });
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
  const prev = useVoice.getState();
  if (
    prev.status === "joined" &&
    prev.serverId &&
    prev.channelId &&
    prev.channelId !== input.channelId
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
  if (useVoice.getState().watchChannelId === input.channelId) {
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
  if (state.live && serverId && channelId) {
    applyLiveEnd(serverId, channelId, currentUserId() ?? undefined);
  }
  stopPeer();
  vacateSelf(serverId, channelId);
  useVoice.setState({
    ...idle,
    watching: state.watching,
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
  if (state.status === "joined" && state.channelId === input.channelId) {
    return;
  }
  if (state.watching && state.watchChannelId === input.channelId) {
    return;
  }
  useVoice.setState({
    watching: true,
    watchChannelId: input.channelId,
    watchStream: null,
  });
  void startWatchPeer(input.channelId);
}

export function stopWatching(): void {
  stopWatchPeer();
  useVoice.setState({
    watching: false,
    watchChannelId: null,
    watchStream: null,
  });
}

export function resetVoiceForTests(): void {
  awaitingJoin = null;
  awaitingLive = null;
  pendingIce = [];
  stopWatchPeer();
  stopPeer();
  useVoice.setState({ ...idle });
  deps = null;
  bound = false;
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
  const mine = generation;
  const getMedia =
    kind === "v"
      ? (deps?.getUserMedia ?? defaultGetUserMedia)
      : (deps?.getDisplayMedia ?? defaultGetDisplayMedia);
  // Video only for camera / screen / live. Display audio mixed into the
  // voice m-line (and tagged as a second "a" pub) was echo-y on deploy and
  // added a video-sized SDP the 12 KiB cap then rejected.
  const constraints: MediaStreamConstraints = { audio: false, video: true };
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
  if (generation !== mine || videoEpoch(kind) !== epoch) {
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
  if (generation !== mine || videoEpoch(kind) !== epoch) {
    return;
  }
  await publishLocal(kind, stream);
}

function stopLocalVideo(kind: "v" | "s" | "l"): void {
  const state = useVoice.getState();
  if (kind === "v" && !state.camera && !state.localCamera) return;
  if (kind === "s" && !state.sharing && !state.localScreen) return;
  if (kind === "l" && !state.live && !state.localLive) return;
  if (kind === "v") cameraEpoch += 1;
  else if (kind === "s") screenEpoch += 1;
  else liveEpoch += 1;
  const stream =
    kind === "v" ? cameraStream : kind === "s" ? screenStream : liveStream;
  const senders = peer?.getSenders?.() ?? [];
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
    if (sender) peer?.removeTrack?.(sender);
    track.stop();
  }
  needOffer = true;
  void offerIfStable(generation);
}

async function publishLocal(
  kind: "v" | "s" | "l",
  stream: MediaStream,
): Promise<void> {
  if (!peer) return;
  const tracks = stream.getVideoTracks();
  if (tracks.length === 0) return;
  media?.send({ op: "p", k: kind });
  for (const track of tracks) {
    peer.addTrack?.(track, stream);
  }
  const self = currentUserId();
  if (self) setPub(self, kind, true);
  sendPub(kind, true);
  needOffer = true;
  await offerIfStable(generation);
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
  if (!parsed) return;
  const attached = stream ?? new MediaStream([track]);
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
  await enqueueSdp(async () => {
    if (generation !== mine || !peer) return;
    if (opts?.initial && sfuOffered) return;
    if (makingOffer || signalingState() !== "stable") {
      // Sticky only for explicit publish/unpublish. negotiationneeded
      // fires again once we are stable (W3C perfect negotiation).
      if (!opts?.initial && !opts?.fromEvent) needOffer = true;
      return;
    }
    makingOffer = true;
    needOffer = false;
    try {
      if (signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) needOffer = true;
        return;
      }
      if (opts?.initial && sfuOffered) return;
      preferOpus(peer);
      preferVp8(peer);
      const offer = withTunedSdp(await peer.createOffer());
      if (generation !== mine || signalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) needOffer = true;
        return;
      }
      await peer.setLocalDescription(offer);
      if (generation !== mine || !offer.sdp) return;
      media?.send({ op: "o", sdp: offer.sdp });
    } catch (error) {
      if (generation === mine) {
        deps?.onError?.(error);
      }
    } finally {
      makingOffer = false;
    }
  });
}

async function startPeer(serverId: string, channelId: string): Promise<void> {
  const mine = generation + 1;
  const resumeCamera = useVoice.getState().camera;
  const resumeShare = useVoice.getState().sharing;
  const resumeLive = useVoice.getState().live;
  stopPeer();
  generation = mine;
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
    if (generation !== mine) return;
    iceServers = ticket.ice_servers ?? [];
    const socket = openMedia(mediaWsUrl(ticket.media_path));
    media = socket;
    unbindMedia = socket.onFrame(onMediaFrame);
    socket.send({ op: "j", tk: ticket.ticket });
  } catch (error) {
    if (generation !== mine) return;
    rollbackSeat(error);
    return;
  }

  const pc = createPeer(iceServers);
  peer = pc;

  pc.onnegotiationneeded = () => {
    if (generation !== mine) return;
    void offerIfStable(mine, { fromEvent: true });
  };
  pc.onicecandidate = (event) => {
    if (generation !== mine) return;
    if (!event.candidate) return;
    media?.send({
      op: "i",
      ice: event.candidate.candidate,
      ...(event.candidate.sdpMid ? { mid: event.candidate.sdpMid } : {}),
    });
  };
  pc.ontrack = (event) => {
    if (generation !== mine) return;
    attachIncoming(event.track, event.streams[0]);
  };

  try {
    const stream = await getUserMedia({ audio: MIC_AUDIO, video: false });
    if (generation !== mine) {
      stream.getTracks().forEach((track) => track.stop());
      return;
    }
    localStream = stream;
    stream.getAudioTracks().forEach((track) => hintTrack(track, "speech"));
    applyLocalAudio();
    for (const track of stream.getTracks()) {
      pc.addTrack?.(track, stream);
    }
    preferOpus(pc);
    preferVp8(pc);
    const self = currentUserId();
    if (self && useVoice.getState().channelId === channelId) {
      setPub(self, "a", true);
    }
    deps?.gateway.send({
      op: "sig",
      t: "p",
      s: serverId,
      c: channelId,
      k: "a",
    });
  } catch {
    // No mic — still send an offer so ICE can run.
  }

  if (generation !== mine) return;
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
  if (generation !== mine) return;
  await offerIfStable(mine, { initial: true });
}

function stopWatchPeer(): void {
  watchGeneration += 1;
  watchPendingIce = [];
  watchSdpChain = Promise.resolve();
  watchMakingOffer = false;
  watchSfuOffered = false;
  watchNeedOffer = false;
  unbindWatch?.();
  unbindWatch = null;
  watchMedia?.close();
  watchMedia = null;
  watchPeer?.close();
  watchPeer = null;
  if (watchAudio) {
    watchAudio.srcObject = null;
  }
}

function enqueueWatchSdp(job: () => Promise<void>): Promise<void> {
  const run = watchSdpChain.then(job, job);
  watchSdpChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

function watchSignalingState(): string {
  return (
    watchPeer?.signalingState ??
    (watchPeer?.remoteDescription ? "have-remote-offer" : "stable")
  );
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
    return;
  }
  const parsed = parseIncomingVideo(track, stream);
  const attached = stream ?? new MediaStream([track]);
  const state = useVoice.getState();
  if (!parsed) {
    // Recvonly watch: SFU msid may be missing. Any video is the live tile.
    useVoice.setState({ watchStream: attached });
    return;
  }
  const current = state.remote[parsed.userId] ?? {};
  useVoice.setState({
    watchStream:
      parsed.k === "l" || !state.watchStream ? attached : state.watchStream,
    remote: {
      ...state.remote,
      [parsed.userId]: { ...current, [parsed.k]: attached },
    },
  });
}

async function applyWatchRemote(
  type: "offer" | "answer",
  sdp: string,
): Promise<void> {
  if (!watchPeer) return;
  if (type === "offer") {
    const collision = watchMakingOffer || watchSignalingState() !== "stable";
    if (collision) {
      watchNeedOffer = true;
      try {
        await watchPeer.setLocalDescription({ type: "rollback" });
      } catch {
        // implicit rollback
      }
    }
    watchSfuOffered = true;
  }
  await watchPeer.setRemoteDescription({ type, sdp: tuneAudioSdp(sdp) });
  const queued = watchPendingIce;
  watchPendingIce = [];
  for (const candidate of queued) {
    await watchPeer.addIceCandidate(candidate);
  }
  if (type === "offer") {
    const answer = withTunedSdp(await watchPeer.createAnswer());
    await watchPeer.setLocalDescription(answer);
    if (answer.sdp) {
      watchMedia?.send({ op: "a", sdp: answer.sdp });
    }
  }
  if (watchNeedOffer) {
    void watchOfferIfStable(watchGeneration);
  }
}

function onWatchFrame(frame: MediaServerFrame): void {
  if (frame.op === "err") {
    stopWatching();
    const code: ApiErrorCode =
      frame.e === "unauthorized" ? "unauthenticated" : "bad_request";
    deps?.onError?.(new ApiError(code, 0, errorMessage(code)));
    return;
  }
  if (frame.op === "a" && frame.sdp) {
    void enqueueWatchSdp(() => applyWatchRemote("answer", frame.sdp));
    return;
  }
  if (frame.op === "o" && frame.sdp) {
    void enqueueWatchSdp(() => applyWatchRemote("offer", frame.sdp));
    return;
  }
  if (frame.op === "i" && frame.ice) {
    const candidate = { candidate: frame.ice, sdpMid: frame.mid ?? null };
    if (watchPeer?.remoteDescription) {
      void watchPeer.addIceCandidate(candidate);
    } else {
      watchPendingIce.push(candidate);
    }
  }
}

async function watchOfferIfStable(
  mine: number,
  opts?: { initial?: boolean; fromEvent?: boolean },
): Promise<void> {
  await enqueueWatchSdp(async () => {
    if (watchGeneration !== mine || !watchPeer) return;
    if (opts?.initial && watchSfuOffered) return;
    if (watchMakingOffer || watchSignalingState() !== "stable") {
      if (!opts?.initial && !opts?.fromEvent) watchNeedOffer = true;
      return;
    }
    watchMakingOffer = true;
    watchNeedOffer = false;
    try {
      if (watchSignalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) watchNeedOffer = true;
        return;
      }
      if (opts?.initial && watchSfuOffered) return;
      preferOpus(watchPeer);
      preferVp8(watchPeer);
      const offer = withTunedSdp(await watchPeer.createOffer());
      if (watchGeneration !== mine || watchSignalingState() !== "stable") {
        if (!opts?.initial && !opts?.fromEvent) watchNeedOffer = true;
        return;
      }
      await watchPeer.setLocalDescription(offer);
      if (watchGeneration !== mine || !offer.sdp) return;
      watchMedia?.send({ op: "o", sdp: offer.sdp });
    } catch (error) {
      if (watchGeneration === mine) {
        deps?.onError?.(error);
      }
    } finally {
      watchMakingOffer = false;
    }
  });
}

async function startWatchPeer(channelId: string): Promise<void> {
  const mine = watchGeneration + 1;
  stopWatchPeer();
  watchGeneration = mine;
  const fetchTicket = deps?.fetchTicket ?? requestMediaTicket;
  const openMedia = deps?.openMedia ?? openMediaSocket;
  const createPeer = deps?.createPeer ?? defaultCreatePeer;

  let iceServers: IceServer[];
  try {
    const ticket = await fetchTicket(channelId);
    if (watchGeneration !== mine) return;
    iceServers = ticket.ice_servers ?? [];
    const socket = openMedia(mediaWsUrl(ticket.media_path));
    watchMedia = socket;
    unbindWatch = socket.onFrame(onWatchFrame);
    socket.send({ op: "j", tk: ticket.ticket });
  } catch (error) {
    if (watchGeneration !== mine) return;
    stopWatching();
    deps?.onError?.(error);
    return;
  }

  const pc = createPeer(iceServers);
  watchPeer = pc;
  pc.onnegotiationneeded = () => {
    if (watchGeneration !== mine) return;
    void watchOfferIfStable(mine, { fromEvent: true });
  };
  // Recvonly m-lines so the offer carries ice-ufrag. webrtc-rs rejects
  // an empty offer with "set_remote_description called with no ice-ufrag".
  pc.addTransceiver?.("audio", { direction: "recvonly" });
  pc.addTransceiver?.("video", { direction: "recvonly" });
  pc.onicecandidate = (event) => {
    if (watchGeneration !== mine) return;
    if (!event.candidate) return;
    watchMedia?.send({
      op: "i",
      ice: event.candidate.candidate,
      ...(event.candidate.sdpMid ? { mid: event.candidate.sdpMid } : {}),
    });
  };
  pc.ontrack = (event) => {
    if (watchGeneration !== mine) return;
    attachWatchIncoming(event.track, event.streams[0]);
  };

  if (watchGeneration !== mine) return;
  await watchOfferIfStable(mine, { initial: true });
}
