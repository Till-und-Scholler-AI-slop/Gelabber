import { afterEach, describe, expect, it, vi } from "vitest";

import type { ClientFrame, ErrFrame, SigEvent } from "../ws/protocol.ts";
import type { IceServer, MediaClientFrame, MediaServerFrame, MediaSocket } from "./media.ts";
import {
  configureVoice,
  joinVoice,
  leaveVoice,
  resetVoiceForTests,
  useVoice,
  type PeerConnection,
} from "./session.ts";

class FakePeer implements PeerConnection {
  onicecandidate: PeerConnection["onicecandidate"] = null;
  ontrack: PeerConnection["ontrack"] = null;
  remoteDescription: { type: string } | null = null;
  signalingState = "stable";
  closed = false;
  tracks = 0;
  ice: { candidate: string; sdpMid: string | null }[] = [];
  iceServers: IceServer[];

  constructor(iceServers: IceServer[] = []) {
    this.iceServers = iceServers;
  }

  addTrack(): void {
    this.tracks += 1;
  }

  async createOffer(): Promise<{ type: string; sdp?: string }> {
    if (this.signalingState !== "stable") {
      throw new Error("InvalidStateError");
    }
    return { type: "offer", sdp: "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n" };
  }

  async createAnswer(): Promise<{ type: string; sdp?: string }> {
    return { type: "answer", sdp: "v=0\r\no=- 2 2 IN IP4 127.0.0.1\r\n" };
  }

  async setLocalDescription(desc?: { type: string }): Promise<void> {
    if (desc?.type === "offer") this.signalingState = "have-local-offer";
    if (desc?.type === "answer") this.signalingState = "stable";
    this.onicecandidate?.({
      candidate: { candidate: "candidate:1 1 UDP 1 127.0.0.1 9 typ host", sdpMid: "0" },
    });
    this.onicecandidate?.({ candidate: null });
  }

  async setRemoteDescription(desc: { type: string }): Promise<void> {
    this.remoteDescription = desc;
    this.signalingState = desc.type === "offer" ? "have-remote-offer" : "stable";
  }

  async addIceCandidate(candidate: {
    candidate: string;
    sdpMid: string | null;
  }): Promise<void> {
    this.ice.push(candidate);
  }

  close(): void {
    this.closed = true;
  }
}

function fakeStream(): MediaStream {
  const track = {
    stop() {},
  } as MediaStreamTrack;
  return {
    getTracks: () => [track],
  } as MediaStream;
}

function install(opts?: {
  media?: boolean;
  userId?: string;
  ticketFail?: boolean;
  holdMedia?: Promise<void>;
}) {
  const sent: ClientFrame[] = [];
  const mediaSent: MediaClientFrame[] = [];
  let onSig: ((event: SigEvent) => void) | undefined;
  let onErr: ((err: ErrFrame) => void) | undefined;
  let onReady: (() => void) | undefined;
  let onMedia: ((frame: MediaServerFrame) => void) | undefined;
  const peers: FakePeer[] = [];
  const errors: unknown[] = [];

  configureVoice({
    userId: () => opts?.userId ?? "u-self",
    gateway: {
      send: (frame) => sent.push(frame),
      onSig: (listener) => {
        onSig = listener;
        return () => {
          onSig = undefined;
        };
      },
      onErr: (listener) => {
        onErr = listener;
        return () => {
          onErr = undefined;
        };
      },
      onReady: (listener) => {
        onReady = listener;
        return () => {
          onReady = undefined;
        };
      },
    },
    createPeer: (iceServers) => {
      const peer = new FakePeer(iceServers);
      peers.push(peer);
      return peer;
    },
    getUserMedia: async () => {
      if (opts?.holdMedia) await opts.holdMedia;
      if (opts?.media === false) throw new Error("denied");
      return fakeStream();
    },
    fetchTicket: async () => {
      if (opts?.ticketFail) throw new Error("ticket");
      return {
        ticket: "abcdefghjkmn",
        media_path: "/media/ws",
        ice_servers: [{ urls: ["stun:127.0.0.1:3478"] }],
      };
    },
    openMedia: () => {
      const socket: MediaSocket = {
        send: (frame) => mediaSent.push(frame),
        close() {},
        onFrame(handler) {
          onMedia = handler;
          return () => {
            onMedia = undefined;
          };
        },
      };
      return socket;
    },
    onError: (error) => errors.push(error),
  });

  return {
    sent,
    mediaSent,
    peers,
    errors,
    emitSig: (event: SigEvent) => onSig?.(event),
    emitErr: (err: ErrFrame) => onErr?.(err),
    emitReady: () => onReady?.(),
    emitMedia: (frame: MediaServerFrame) => onMedia?.(frame),
  };
}

describe("voice session", () => {
  afterEach(() => {
    resetVoiceForTests();
  });

  it("join click sets local state before any ICE work", () => {
    const { sent, mediaSent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });

    const state = useVoice.getState();
    expect(state.status).toBe("joined");
    expect(state.channelId).toBe("voice");
    expect(state.participants["u-self"]).toEqual({ pubs: [] });
    expect(sent[0]).toEqual({ op: "sig", t: "j", s: "srv", c: "voice" });
    expect(sent.some((frame) => frame.op === "sig" && frame.t === "o")).toBe(
      false,
    );
    expect(mediaSent.some((frame) => frame.op === "o")).toBe(false);
  });

  it("sends offer and ice on the media WS, pub on chat WS", async () => {
    const { sent, mediaSent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => {
      expect(mediaSent.some((frame) => frame.op === "o")).toBe(true);
    });
    expect(peers).toHaveLength(1);
    expect(peers[0]?.tracks).toBe(1);
    expect(peers[0]?.iceServers[0]?.urls).toEqual(["stun:127.0.0.1:3478"]);
    expect(mediaSent.map((frame) => frame.op)).toEqual(["j", "i", "o"]);
    expect(mediaSent[0]).toEqual({ op: "j", tk: "abcdefghjkmn" });
    expect(sent.filter((frame) => frame.op === "sig").map((frame) => frame.t)).toEqual(
      ["j", "p"],
    );
    expect(sent.some((frame) => frame.op === "sig" && frame.t === "o")).toBe(
      false,
    );
  });

  it("rolls back the seat when the media ticket fails", async () => {
    const { sent, errors } = install({ ticketFail: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    expect(useVoice.getState().status).toBe("joined");
    await vi.waitFor(() => expect(useVoice.getState().status).toBe("idle"));
    expect(errors).toHaveLength(1);
    expect(sent.at(-1)).toEqual({ op: "sig", t: "l", s: "srv", c: "voice" });
  });

  it("rolls back the seat on a media-path err", async () => {
    const { emitMedia, errors, sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(useVoice.getState().status).toBe("joined"));
    emitMedia({ op: "err", e: "unauthorized" });
    expect(useVoice.getState().status).toBe("idle");
    expect(errors).toHaveLength(1);
    expect(sent.at(-1)).toEqual({ op: "sig", t: "l", s: "srv", c: "voice" });
  });

  it("answers an SFU offer instead of offering when the SFU spoke first", async () => {
    let release!: () => void;
    const holdMedia = new Promise<void>((resolve) => {
      release = resolve;
    });
    const { mediaSent, emitMedia, peers } = install({ holdMedia });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitMedia({ op: "o", sdp: "v=0\r\no=- 9 9 IN IP4 127.0.0.1\r\n" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "a")).toBe(true),
    );
    release();
    await vi.waitFor(() => expect(peers[0]?.tracks).toBe(1));
    expect(mediaSent.some((frame) => frame.op === "o")).toBe(false);
    expect(peers[0]?.signalingState).not.toBe("have-local-offer");
  });

  it("rolls back local join when the server forbids it", () => {
    const { emitErr, errors } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    emitErr({ op: "err", e: "forbidden", s: "srv", c: "voice" });
    expect(useVoice.getState().status).toBe("idle");
    expect(errors).toHaveLength(1);
  });

  it("tracks other participants from sig frames, not chat events", () => {
    const { emitSig } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-bob" });
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-bob",
      k: "a",
    });
    expect(useVoice.getState().participants["u-bob"]).toEqual({ pubs: ["a"] });
    emitSig({ op: "sig", t: "l", s: "srv", c: "voice", u: "u-bob" });
    expect(useVoice.getState().participants["u-bob"]).toBeUndefined();
  });

  it("leave clears local state immediately and closes the peer", async () => {
    const { sent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    leaveVoice();
    expect(useVoice.getState().status).toBe("idle");
    expect(peers[0]?.closed).toBe(true);
    expect(sent.at(-1)).toEqual({ op: "sig", t: "l", s: "srv", c: "voice" });
  });

  it("never sends a product token on the chat socket", () => {
    const { sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    for (const frame of sent) {
      expect(frame).not.toHaveProperty("token");
      expect(frame).not.toHaveProperty("identity");
      expect(frame.op).toBe("sig");
    }
  });

  it("queues trickle ICE from the media path until the remote answer", async () => {
    const { peers, emitMedia } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitMedia({
      op: "i",
      ice: "candidate:1 1 UDP 1 127.0.0.1 9 typ host",
      mid: "0",
    });
    expect(peers[0]?.ice).toEqual([]);
    emitMedia({
      op: "a",
      sdp: "v=0\r\n",
    });
    await vi.waitFor(() => expect(peers[0]?.ice).toHaveLength(1));
    expect(peers[0]?.remoteDescription?.type).toBe("answer");
    expect(peers[0]?.ice[0]?.candidate).toContain("candidate:1");
  });

  it("answers an SFU renegotiation offer on the media path", async () => {
    const { peers, mediaSent, emitMedia } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitMedia({ op: "o", sdp: "v=0\r\no=- 9 9 IN IP4 127.0.0.1\r\n" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "a")).toBe(true),
    );
    expect(peers[0]?.remoteDescription?.type).toBe("offer");
  });

  it("does not leave the seat on a later channel bad_request", () => {
    const { emitErr, emitSig, errors } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    emitErr({ op: "err", e: "bad_request", s: "srv", c: "voice" });
    emitErr({ op: "err", e: "bad_request" });
    expect(useVoice.getState().status).toBe("joined");
    expect(errors).toHaveLength(0);
  });

  it("drops users who left while the socket was down", () => {
    const { emitSig, emitReady } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-bob" });
    expect(useVoice.getState().participants["u-bob"]).toBeDefined();
    emitReady();
    expect(useVoice.getState().participants["u-bob"]).toBeUndefined();
    expect(useVoice.getState().participants["u-self"]).toBeDefined();
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-cara" });
    expect(useVoice.getState().participants["u-cara"]).toBeDefined();
    expect(useVoice.getState().participants["u-bob"]).toBeUndefined();
  });
});
