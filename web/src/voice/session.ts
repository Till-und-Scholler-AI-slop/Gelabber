// Local voice state + native RTCPeerConnection (issue #10).
// Join updates the store immediately; ICE / getUserMedia run afterwards.
// Own protocol only — no product video SDK, no foreign media JWT.

import { create } from "zustand";

import { ApiError, type ApiErrorCode } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { useSession } from "../auth/session.ts";
import { notifyError } from "../components/toasts.ts";
import { getGateway, type Gateway } from "../ws/client.ts";
import type { ErrFrame, SigEvent, TrackKind } from "../ws/protocol.ts";

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
  addTrack?(track: MediaStreamTrack, stream: MediaStream): void;
  createOffer(): Promise<{ type: string; sdp?: string }>;
  setLocalDescription(desc: { type: string; sdp?: string }): Promise<void>;
  setRemoteDescription(desc: { type: string; sdp?: string }): Promise<void>;
  addIceCandidate(candidate: {
    candidate: string;
    sdpMid: string | null;
  }): Promise<void>;
  close(): void;
  remoteDescription?: { type: string } | null;
};

export type VoiceGateway = Pick<Gateway, "send" | "onSig" | "onErr" | "onReady">;

export type VoiceDeps = {
  gateway: VoiceGateway;
  userId: () => string | null;
  createPeer: () => PeerConnection;
  getUserMedia: (constraints: MediaStreamConstraints) => Promise<MediaStream>;
  onError?: (error: unknown) => void;
};

let deps: VoiceDeps | null = null;
let peer: PeerConnection | null = null;
let localStream: MediaStream | null = null;
let bound = false;
let generation = 0;

function currentUserId(): string | null {
  return deps?.userId && deps.userId !== currentUserId
    ? deps.userId()
    : useSession.getState().user?.id ?? null;
}

function defaultCreatePeer(): PeerConnection {
  return new RTCPeerConnection({ iceServers: [] }) as unknown as PeerConnection;
}

async function defaultGetUserMedia(
  constraints: MediaStreamConstraints,
): Promise<MediaStream> {
  return navigator.mediaDevices.getUserMedia(constraints);
}

export function configureVoice(next: Partial<VoiceDeps>): void {
  const gateway = next.gateway ?? deps?.gateway ?? getGateway();
  deps = {
    gateway,
    userId: next.userId ?? deps?.userId ?? currentUserId,
    createPeer: next.createPeer ?? deps?.createPeer ?? defaultCreatePeer,
    getUserMedia: next.getUserMedia ?? deps?.getUserMedia ?? defaultGetUserMedia,
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
    case "a":
      if (event.u !== currentUserId() && event.sdp && peer) {
        void peer.setRemoteDescription({ type: "answer", sdp: event.sdp });
      }
      return;
    case "i":
      if (
        event.u !== currentUserId() &&
        event.ice &&
        peer?.remoteDescription
      ) {
        void peer.addIceCandidate({
          candidate: event.ice,
          sdpMid: event.mid ?? null,
        });
      }
      return;
    default:
      return;
  }
}

function onErr(err: ErrFrame): void {
  const state = useVoice.getState();
  if (state.status === "idle") return;
  if (err.c && state.channelId && err.c !== state.channelId) return;
  if (err.e !== "forbidden" && err.e !== "not_found" && err.e !== "bad_request") {
    return;
  }
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
  peer?.close();
  peer = null;
  localStream?.getTracks().forEach((track) => track.stop());
  localStream = null;
}

/**
 * Join a voice channel. Local state flips first so the click feels instant;
 * the socket frame and ICE run after.
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
  stopPeer();
  useVoice.setState({ ...idle });
  deps = null;
  bound = false;
}

async function startPeer(serverId: string, channelId: string): Promise<void> {
  const mine = generation + 1;
  stopPeer();
  generation = mine;
  const createPeer = deps?.createPeer ?? defaultCreatePeer;
  const getUserMedia = deps?.getUserMedia ?? defaultGetUserMedia;
  const pc = createPeer();
  peer = pc;

  pc.onicecandidate = (event) => {
    if (generation !== mine) return;
    if (!event.candidate) return;
    deps?.gateway.send({
      op: "sig",
      t: "i",
      s: serverId,
      c: channelId,
      ice: event.candidate.candidate,
      ...(event.candidate.sdpMid ? { mid: event.candidate.sdpMid } : {}),
    });
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
  try {
    const offer = await pc.createOffer();
    if (generation !== mine) return;
    await pc.setLocalDescription(offer);
    if (generation !== mine || !offer.sdp) return;
    deps?.gateway.send({
      op: "sig",
      t: "o",
      s: serverId,
      c: channelId,
      sdp: offer.sdp,
    });
  } catch (error) {
    deps?.onError?.(error);
  }
}
