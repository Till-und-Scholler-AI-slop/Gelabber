import { afterEach, describe, expect, it, vi } from "vitest";

import type { ClientFrame, ErrFrame, SigEvent } from "../ws/protocol.ts";
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
  remoteDescription: { type: string } | null = null;
  closed = false;
  tracks = 0;
  ice: { candidate: string; sdpMid: string | null }[] = [];

  addTrack(): void {
    this.tracks += 1;
  }

  async createOffer(): Promise<{ type: string; sdp?: string }> {
    return { type: "offer", sdp: "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n" };
  }

  async setLocalDescription(): Promise<void> {
    this.onicecandidate?.({
      candidate: { candidate: "candidate:1 1 UDP 1 127.0.0.1 9 typ host", sdpMid: "0" },
    });
    this.onicecandidate?.({ candidate: null });
  }

  async setRemoteDescription(desc: { type: string }): Promise<void> {
    this.remoteDescription = desc;
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

function install(opts?: { media?: boolean; userId?: string }) {
  const sent: ClientFrame[] = [];
  let onSig: ((event: SigEvent) => void) | undefined;
  let onErr: ((err: ErrFrame) => void) | undefined;
  let onReady: (() => void) | undefined;
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
    createPeer: () => {
      const peer = new FakePeer();
      peers.push(peer);
      return peer;
    },
    getUserMedia: async () => {
      if (opts?.media === false) throw new Error("denied");
      return fakeStream();
    },
    onError: (error) => errors.push(error),
  });

  return {
    sent,
    peers,
    errors,
    emitSig: (event: SigEvent) => onSig?.(event),
    emitErr: (err: ErrFrame) => onErr?.(err),
    emitReady: () => onReady?.(),
  };
}

describe("voice session", () => {
  afterEach(() => {
    resetVoiceForTests();
  });

  it("join click sets local state before any ICE work", () => {
    const { sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });

    const state = useVoice.getState();
    expect(state.status).toBe("joined");
    expect(state.channelId).toBe("voice");
    expect(state.participants["u-self"]).toEqual({ pubs: [] });
    expect(sent[0]).toEqual({ op: "sig", t: "j", s: "srv", c: "voice" });
    expect(sent.some((frame) => frame.op === "sig" && frame.t === "o")).toBe(
      false,
    );
  });

  it("sends offer, ice and audio pub in the background via native PC", async () => {
    const { sent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => {
      expect(sent.some((frame) => frame.op === "sig" && frame.t === "o")).toBe(
        true,
      );
    });
    expect(peers).toHaveLength(1);
    expect(peers[0]?.tracks).toBe(1);
    expect(
      sent.filter((frame) => frame.op === "sig").map((frame) => frame.t),
    ).toEqual(["j", "p", "i", "o"]);
    const offer = sent.find((frame) => frame.op === "sig" && frame.t === "o");
    expect(offer && "sdp" in offer ? offer.sdp?.startsWith("v=0") : false).toBe(
      true,
    );
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

  it("never sends a media token or product envelope", () => {
    const { sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    for (const frame of sent) {
      expect(frame).not.toHaveProperty("token");
      expect(frame).not.toHaveProperty("identity");
      expect(frame.op).toBe("sig");
    }
  });

  it("queues trickle ICE that arrives before the remote answer", async () => {
    const { peers, emitSig } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitSig({
      op: "sig",
      t: "i",
      s: "srv",
      c: "voice",
      u: "sfu",
      ice: "candidate:1 1 UDP 1 127.0.0.1 9 typ host",
      mid: "0",
    });
    expect(peers[0]?.ice).toEqual([]);
    emitSig({
      op: "sig",
      t: "a",
      s: "srv",
      c: "voice",
      u: "sfu",
      sdp: "v=0\r\n",
    });
    await vi.waitFor(() => expect(peers[0]?.ice).toHaveLength(1));
    expect(peers[0]?.remoteDescription?.type).toBe("answer");
    expect(peers[0]?.ice[0]?.candidate).toContain("candidate:1");
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
