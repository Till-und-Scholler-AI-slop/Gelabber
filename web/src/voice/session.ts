// Local voice state + native RTCPeerConnection.
// Join updates the store immediately; ticket / ICE / getUserMedia run after.
// Chat WS: presence (j/l/p/u). Media WS: SDP/ICE + RTP. No product SDK.

import { create } from "zustand";

import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { useSession } from "../auth/session.ts";
import { notifyError } from "../components/toasts.ts";
import { getGateway, type Gateway } from "../ws/client.ts";
import type { ErrFrame, SigEvent, TrackKind } from "../ws/protocol.ts";
import {
  type IceServer,
  type MediaServerFrame,
  type MediaSocket,
  type OpenMedia,
  mediaWsUrl,
  openMediaSocket,
  requestMediaTicket,
} from "./media.ts";

export type VoiceStatus = "idle" | "joined";

export type VoiceParticipant = {
  pubs: TrackKind[];
};

export type VoiceState = {
  status: VoiceStatus;
  serverId: string | null;
  channelId: string | null;
  channelName: string | null;
  participants: Record<string, VoiceParticipant>;
};

const idle: VoiceState = {
  status: "idle",
  serverId: null,
  channelId: null,
  channelName: null,
  participants: {},
};

export const useVoice = create<VoiceState>(() => ({ ...idle }));

export type PeerConnection = {
  onicecandidate: ((event: {
    candidate: { candidate: string; sdpMid: string | null } | null;
  }) => void) | null;
  ontrack: ((event: { streams: MediaStream[]; track: MediaStreamTrack }) => void) | null;
  addTrack?(track: MediaStreamTrack, stream: MediaStream): void;
  createOffer(): Promise<{ type: string; sdp?: string }>;
  createAnswer(): Promise<{ type: string; sdp?: string }>;
  setLocalDescription(desc: { type: string; sdp?: string }): Promise<void>;
  setRemoteDescription(desc: { type: string; sdp?: string }): Promise<void>;
  addIceCandidate(candidate: {
    candidate: string;
    sdpMid: string | null;
  }): Promise<void>;
  close(): void;
  remoteDescription?: { type: string } | null;
  signalingState?: string;
};

export type VoiceGateway = Pick<Gateway, "send" | "onSig" | "onErr" | "onReady">;

export type VoiceDeps = {
  gateway: VoiceGateway;
  userId: () => string | null;
  createPeer: (iceServers: IceServer[]) => PeerConnection;
  getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
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
let remoteAudio: HTMLAudioElement | null = null;
let bound = false;
let generation = 0;
let pendingIce: IceCand[] = [];
let awaitingJoin: { serverId: string; channelId: string } | null = null;
let sdpChain: Promise<void> = Promise.resolve();
let makingOffer = false;
let sfuOffered = false;

function currentUserId(): string | null {
  return deps?.userId && deps.userId !== currentUserId
    ? deps.userId()
    : useSession.getState().user?.id ?? null;
}

function defaultCreatePeer(iceServers: IceServer[]): PeerConnection {
  return new RTCPeerConnection({ iceServers }) as unknown as PeerConnection;
}

async function defaultGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia(constraints);
}

function defaultAttachRemote(stream: MediaStream): void {
  if (typeof Audio === "undefined") return;
  if (!remoteAudio) {
    remoteAudio = new Audio();
    remoteAudio.autoplay = true;
  }
  remoteAudio.srcObject = stream;
}

export function configureVoice(next: Partial<VoiceDeps>): void {
  const gateway = next.gateway ?? deps?.gateway ?? getGateway();
  deps = {
    gateway,
    userId: next.userId ?? deps?.userId ?? currentUserId,
    createPeer: next.createPeer ?? deps?.createPeer ?? defaultCreatePeer,
    getUserMedia: next.getUserMedia ?? deps?.getUserMedia ?? defaultGetUserMedia,
    fetchTicket: next.fetchTicket ?? deps?.fetchTicket ?? requestMediaTicket,
    openMedia: next.openMedia ?? deps?.openMedia ?? openMediaSocket,
    attachRemote: next.attachRemote ?? deps?.attachRemote ?? defaultAttachRemote,
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

function onSig(event: SigEvent): void {
  const state = useVoice.getState();
  if (state.status === "idle" || event.c !== state.channelId) {
    return;
  }
  switch (event.t) {
    case "j":
      if (
        event.u === currentUserId() &&
        awaitingJoin &&
        event.c === awaitingJoin.channelId
      ) {
        awaitingJoin = null;
      }
      useVoice.setState({
        participants: upsert(state.participants, event.u),
      });
      return;
    case "l":
      if (event.u === currentUserId()) {
        return;
      }
      {
        const next = { ...state.participants };
        delete next[event.u];
        useVoice.setState({ participants: next });
      }
      return;
    case "p":
    case "u": {
      if (!event.k) return;
      const current = state.participants[event.u] ?? { pubs: [] };
      const pubs =
        event.t === "p"
          ? current.pubs.includes(event.k)
            ? current.pubs
            : [...current.pubs, event.k]
          : current.pubs.filter((kind) => kind !== event.k);
      useVoice.setState({
        participants: {
          ...state.participants,
          [event.u]: { pubs },
        },
      });
      return;
    }
    default:
      return;
  }
}

function onErr(err: ErrFrame): void {
  const pending = awaitingJoin;
  if (!pending) return;
  if (err.c !== pending.channelId) return;
  if (err.e !== "forbidden" && err.e !== "not_found" && err.e !== "bad_request") {
    return;
  }
  const state = useVoice.getState();
  if (state.channelId !== pending.channelId) return;
  awaitingJoin = null;
  stopPeer();
  useVoice.setState({ ...idle });
  const code: ApiErrorCode =
    err.e === "forbidden" || err.e === "not_found" || err.e === "bad_request"
      ? err.e
      : "bad_request";
  deps?.onError?.(new ApiError(code, 0, errorMessage(code)));
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
  });
  awaitingJoin = { serverId: state.serverId, channelId: state.channelId };
  deps?.gateway.send({
    op: "sig",
    t: "j",
    s: state.serverId,
    c: state.channelId,
  });
  void startPeer(state.serverId, state.channelId);
}

function stopPeer(): void {
  generation += 1;
  pendingIce = [];
  sdpChain = Promise.resolve();
  makingOffer = false;
  sfuOffered = false;
  unbindMedia?.();
  unbindMedia = null;
  media?.close();
  media = null;
  peer?.close();
  peer = null;
  localStream?.getTracks().forEach((track) => track.stop());
  localStream = null;
  if (remoteAudio) {
    remoteAudio.srcObject = null;
  }
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
  stopPeer();
  useVoice.setState({ ...idle });
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

function signalingState(): string {
  return peer?.signalingState ?? (peer?.remoteDescription ? "have-remote-offer" : "stable");
}

async function applyRemoteDescription(
  type: "offer" | "answer",
  sdp: string,
): Promise<void> {
  if (!peer) return;
  if (type === "offer") {
    sfuOffered = true;
  }
  await peer.setRemoteDescription({ type, sdp });
  const queued = pendingIce;
  pendingIce = [];
  for (const candidate of queued) {
    await peer.addIceCandidate(candidate);
  }
  if (type === "offer") {
    const answer = await peer.createAnswer();
    await peer.setLocalDescription(answer);
    if (answer.sdp) {
      media?.send({ op: "a", sdp: answer.sdp });
    }
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
    deps?.gateway.send({
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
    serverId: input.serverId,
    channelId: input.channelId,
    channelName: input.channelName,
    participants: { [self]: { pubs: [] } },
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
  stopPeer();
  useVoice.setState({ ...idle });
  if (serverId && channelId) {
    deps?.gateway.send({
      op: "sig",
      t: "l",
      s: serverId,
      c: channelId,
    });
  }
}

export function resetVoiceForTests(): void {
  awaitingJoin = null;
  pendingIce = [];
  stopPeer();
  useVoice.setState({ ...idle });
  deps = null;
  bound = false;
}

async function startPeer(serverId: string, channelId: string): Promise<void> {
  const mine = generation + 1;
  stopPeer();
  generation = mine;
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
    const stream = event.streams[0];
    if (stream) {
      (deps?.attachRemote ?? defaultAttachRemote)(stream);
    }
  };

  try {
    const stream = await getUserMedia({ audio: true, video: false });
    if (generation !== mine) {
      stream.getTracks().forEach((track) => track.stop());
      return;
    }
    localStream = stream;
    for (const track of stream.getTracks()) {
      pc.addTrack?.(track, stream);
    }
    const state = useVoice.getState();
    const self = currentUserId();
    if (self && state.channelId === channelId) {
      const current = state.participants[self] ?? { pubs: [] };
      useVoice.setState({
        participants: {
          ...state.participants,
          [self]: {
            pubs: current.pubs.includes("a") ? current.pubs : [...current.pubs, "a"],
          },
        },
      });
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
  await enqueueSdp(async () => {
    if (generation !== mine || !peer) return;
    if (sfuOffered || makingOffer || signalingState() !== "stable") return;
    makingOffer = true;
    try {
      if (sfuOffered || signalingState() !== "stable") return;
      const offer = await pc.createOffer();
      if (generation !== mine || sfuOffered || signalingState() !== "stable") {
        return;
      }
      await pc.setLocalDescription(offer);
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
