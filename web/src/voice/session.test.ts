import { afterEach, describe, expect, it, vi } from "vitest";

import type { ClientFrame, ErrFrame, SigEvent } from "../ws/protocol.ts";
import type {
  IceServer,
  MediaClientFrame,
  MediaServerFrame,
  MediaSocket,
} from "./media.ts";
import {
  configureVoice,
  joinVoice,
  leaveVoice,
  parseRemoteStreamId,
  resetVoiceForTests,
  toggleCamera,
  toggleDeafen,
  toggleGoLive,
  toggleMute,
  toggleShare,
  watchLive,
  stopWatching,
  useVoice,
  type PeerConnection,
  type RtpSender,
} from "./session.ts";
import { resetVoiceRoster, useVoiceRoster, voiceOf } from "./roster.ts";
import { resetMediaSettingsForTests, useMediaSettings } from "./settings.ts";

class FakePeer implements PeerConnection {
  onicecandidate: PeerConnection["onicecandidate"] = null;
  ontrack: PeerConnection["ontrack"] = null;
  onnegotiationneeded: PeerConnection["onnegotiationneeded"] = null;
  remoteDescription: { type: string } | null = null;
  signalingState = "stable";
  closed = false;
  tracks = 0;
  audio: MediaStreamTrack | null = null;
  senders: RtpSender[] = [];
  ice: { candidate: string; sdpMid: string | null }[] = [];
  iceServers: IceServer[];

  constructor(iceServers: IceServer[] = []) {
    this.iceServers = iceServers;
  }

  addTrack(track?: MediaStreamTrack): RtpSender {
    this.tracks += 1;
    const encodings: { maxBitrate?: number }[] = [{}];
    const transactionId = `tx-${this.tracks}`;
    const sender: RtpSender = {
      track: track ?? null,
      replaceTrack: async (next) => {
        sender.track = next;
        if (next && next.kind !== "video") this.audio = next;
      },
      getParameters: () => ({ encodings, transactionId }),
      setParameters: async (params) => {
        if (params.transactionId !== transactionId) {
          throw new Error("InvalidModificationError");
        }
        encodings.splice(0, encodings.length, ...params.encodings);
      },
    };
    this.senders.push(sender);
    if (track && track.kind !== "video") this.audio = track;
    return sender;
  }

  addTransceiver(): void {
    this.tracks += 1;
  }

  removeTrack(sender: { track: MediaStreamTrack | null }): void {
    sender.track = null;
  }

  getSenders(): RtpSender[] {
    return this.senders;
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
    if (desc?.type === "rollback") {
      this.signalingState = "stable";
      return;
    }
    if (desc?.type === "offer") this.signalingState = "have-local-offer";
    if (desc?.type === "answer") this.signalingState = "stable";
    this.onicecandidate?.({
      candidate: {
        candidate: "candidate:1 1 UDP 1 127.0.0.1 9 typ host",
        sdpMid: "0",
      },
    });
    this.onicecandidate?.({ candidate: null });
  }

  async setRemoteDescription(desc: { type: string }): Promise<void> {
    if (desc.type === "offer" && this.signalingState === "have-local-offer") {
      throw new Error("InvalidStateError");
    }
    this.remoteDescription = desc;
    this.signalingState =
      desc.type === "offer" ? "have-remote-offer" : "stable";
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

function fakeTrack(kind: "audio" | "video"): MediaStreamTrack {
  return {
    kind,
    enabled: true,
    id: `${kind}-1`,
    stop() {},
    addEventListener() {},
    removeEventListener() {},
  } as unknown as MediaStreamTrack;
}

function fakeStream(): MediaStream {
  const track = fakeTrack("audio");
  return {
    id: "mic",
    getTracks: () => [track],
    getAudioTracks: () => [track],
    getVideoTracks: () => [],
  } as unknown as MediaStream;
}

function fakeVideoStream(id: string, withAudio = false): MediaStream {
  const video = fakeTrack("video");
  const audio = fakeTrack("audio");
  const tracks = withAudio ? [video, audio] : [video];
  return {
    id,
    getTracks: () => tracks,
    getAudioTracks: () => (withAudio ? [audio] : []),
    getVideoTracks: () => [video],
  } as unknown as MediaStream;
}

function install(opts?: {
  media?: boolean;
  display?: boolean;
  userId?: string;
  ticketFail?: boolean;
  holdMedia?: Promise<void>;
  holdDisplay?: Promise<void>;
}) {
  const sent: ClientFrame[] = [];
  const mediaSent: MediaClientFrame[] = [];
  let onSig: ((event: SigEvent) => void) | undefined;
  let onErr: ((err: ErrFrame) => void) | undefined;
  let onReady: (() => void) | undefined;
  let onMedia: ((frame: MediaServerFrame) => void) | undefined;
  const peers: FakePeer[] = [];
  const errors: unknown[] = [];
  let getUserMediaCalls = 0;
  let getDisplayMediaCalls = 0;
  let lastUserMedia: MediaStreamConstraints | undefined;

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
    getUserMedia: async (constraints) => {
      getUserMediaCalls += 1;
      lastUserMedia = constraints;
      if (opts?.holdMedia) await opts.holdMedia;
      if (opts?.media === false) throw new Error("denied");
      if (constraints.video) return fakeVideoStream("local-cam");
      return fakeStream();
    },
    getDisplayMedia: async () => {
      getDisplayMediaCalls += 1;
      if (opts?.holdDisplay) await opts.holdDisplay;
      if (opts?.display === false) throw new Error("denied");
      return fakeVideoStream("local-scr", true);
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
    getUserMediaCalls: () => getUserMediaCalls,
    getDisplayMediaCalls: () => getDisplayMediaCalls,
    lastUserMedia: () => lastUserMedia,
  };
}

describe("voice session", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
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
    expect(
      sent.filter((frame) => frame.op === "sig").map((frame) => frame.t),
    ).toEqual(["j", "p"]);
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

  it("captures voice with echo cancellation on a single channel", async () => {
    const { lastUserMedia, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    expect(lastUserMedia()).toEqual({
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: true,
        channelCount: 1,
      },
      video: false,
    });
  });

  it("answers a colliding SFU offer instead of throwing InvalidStateError", async () => {
    const { peers, mediaSent, emitMedia, errors } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(peers[0]?.signalingState).toBe("have-local-offer"),
    );
    const offersBefore = mediaSent.filter((frame) => frame.op === "o").length;
    emitMedia({ op: "o", sdp: "v=0\r\no=- 9 9 IN IP4 127.0.0.1\r\n" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "a")).toBe(true),
    );
    expect(errors).toHaveLength(0);
    expect(peers[0]?.remoteDescription?.type).toBe("offer");
    await vi.waitFor(() =>
      expect(
        mediaSent.filter((frame) => frame.op === "o").length,
      ).toBeGreaterThan(offersBefore),
    );
  });

  it("re-offers camera after rolling back a colliding SFU offer", async () => {
    const { peers, mediaSent, emitMedia } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(peers[0]?.signalingState).toBe("have-local-offer"),
    );
    emitMedia({ op: "a", sdp: "v=0\r\no=- 2 2 IN IP4 127.0.0.1\r\n" });
    await vi.waitFor(() => expect(peers[0]?.signalingState).toBe("stable"));
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    await vi.waitFor(() =>
      expect(mediaSent.filter((frame) => frame.op === "o").length).toBe(2),
    );
    emitMedia({ op: "o", sdp: "v=0\r\no=- 9 9 IN IP4 127.0.0.1\r\n" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "a")).toBe(true),
    );
    expect(useVoice.getState().camera).toBe(true);
    expect(useVoice.getState().localCamera).toBeTruthy();
    const videoSenders = peers[0]?.senders.filter(
      (sender) => sender.track?.kind === "video",
    );
    expect(videoSenders?.some((sender) => sender.track)).toBe(true);
    await vi.waitFor(() =>
      expect(mediaSent.filter((frame) => frame.op === "o").length).toBe(3),
    );
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

  it("toggles mute immediately, then sends the sync frame", async () => {
    const { sent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    const before = sent.length;
    toggleMute();
    expect(useVoice.getState().muted).toBe(true);
    expect(useVoice.getState().deafened).toBe(false);
    expect(peers[0]?.audio?.enabled).toBe(false);
    expect(
      voiceOf(useVoiceRoster.getState().byServer, "srv", "u-self")?.muted,
    ).toBe(true);
    expect(sent.slice(before)).toEqual([
      { op: "sig", t: "m", s: "srv", c: "voice", on: true },
    ]);
  });

  it("deafens immediately and mutes the session without waiting", async () => {
    const { sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    const before = sent.length;
    toggleDeafen();
    expect(useVoice.getState().deafened).toBe(true);
    expect(useVoice.getState().muted).toBe(true);
    expect(sent.slice(before)).toEqual([
      { op: "sig", t: "m", s: "srv", c: "voice", on: true },
      { op: "sig", t: "d", s: "srv", c: "voice", on: true },
    ]);
    toggleDeafen();
    expect(useVoice.getState().deafened).toBe(false);
    expect(useVoice.getState().muted).toBe(false);
  });

  it("parses SFU stream ids for camera and screen tiles", () => {
    expect(
      parseRemoteStreamId("550e8400-e29b-41d4-a716-446655440000:v"),
    ).toEqual({
      userId: "550e8400-e29b-41d4-a716-446655440000",
      k: "v",
    });
    expect(parseRemoteStreamId("u-bob:s")).toEqual({ userId: "u-bob", k: "s" });
    expect(parseRemoteStreamId("u-bob:l-4097")).toEqual({
      userId: "u-bob",
      k: "l",
    });
    expect(parseRemoteStreamId("u-bob:a")).toBeNull();
    expect(parseRemoteStreamId("livekit")).toBeNull();
  });

  it("shows a local camera preview before any publish offer", async () => {
    const { sent, mediaSent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleCamera();
    expect(useVoice.getState().camera).toBe(true);
    expect(useVoice.getState().localCamera).toBeNull();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    expect(useVoice.getState().participants["u-self"]?.pubs).toContain("v");
    expect(sent.some((frame) => "sdp" in frame && frame.sdp)).toBe(false);
    expect(
      sent.filter((frame) => frame.op === "sig" && frame.t === "p"),
    ).toEqual(
      expect.arrayContaining([
        { op: "sig", t: "p", s: "srv", c: "voice", k: "a" },
        { op: "sig", t: "p", s: "srv", c: "voice", k: "v" },
      ]),
    );
    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "p" && frame.k === "v"),
      ).toBe(true),
    );
  });

  it("starts screen-share without putting SDP on the chat socket", async () => {
    const { sent, mediaSent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleShare();
    expect(useVoice.getState().sharing).toBe(true);
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    expect(useVoice.getState().participants["u-self"]?.pubs).toContain("s");
    for (const frame of sent) {
      expect(frame).not.toHaveProperty("sdp");
      expect(frame).not.toHaveProperty("token");
    }
    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "p" && frame.k === "s"),
      ).toBe(true),
    );
    const videoSenders = peers[0]?.senders.filter(
      (sender) => sender.track?.kind === "video",
    );
    const audioSenders = peers[0]?.senders.filter(
      (sender) => sender.track && sender.track.kind !== "video",
    );
    expect(videoSenders).toHaveLength(1);
    expect(audioSenders).toHaveLength(1);
  });

  it("stops camera locally first, then unpubs", async () => {
    const { sent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    const before = sent.length;
    toggleCamera();
    expect(useVoice.getState().camera).toBe(false);
    expect(useVoice.getState().localCamera).toBeNull();
    expect(sent.slice(before)).toEqual([
      { op: "sig", t: "u", s: "srv", c: "voice", k: "v" },
    ]);
  });

  it("ignores camera and share while not in a channel", () => {
    install();
    toggleCamera();
    toggleShare();
    expect(useVoice.getState().camera).toBe(false);
    expect(useVoice.getState().sharing).toBe(false);
  });

  it("attaches a remote camera tile from the SFU stream id", async () => {
    const { peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    const stream = fakeVideoStream("u-bob:v");
    peers[0]?.ontrack?.({
      track: stream.getVideoTracks()[0]!,
      streams: [stream],
    });
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(stream);
    const tagged = fakeVideoStream("ignored");
    const track = {
      ...fakeTrack("video"),
      id: "u-cara:s-77",
    } as MediaStreamTrack;
    peers[0]?.ontrack?.({ track, streams: [tagged] });
    expect(useVoice.getState().remote["u-cara"]?.s).toBe(tagged);
  });

  it("shows the Live badge immediately and publishes after display capture", async () => {
    const { sent, mediaSent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleGoLive();
    expect(useVoice.getState().live).toBe(true);
    expect(useVoice.getState().localLive).toBeNull();
    expect(useVoiceRoster.getState().live.srv?.voice).toBe("u-self");
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    expect(useVoice.getState().participants["u-self"]?.pubs).toContain("l");
    expect(sent.some((frame) => "sdp" in frame && frame.sdp)).toBe(false);
    expect(
      sent.filter(
        (frame) =>
          frame.op === "sig" &&
          frame.t === "p" &&
          "k" in frame &&
          frame.k === "l",
      ),
    ).toEqual([{ op: "sig", t: "p", s: "srv", c: "voice", k: "l" }]);
    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "p" && frame.k === "l"),
      ).toBe(true),
    );
  });

  it("rolls back Go Live locally when the server forbids it", async () => {
    const { emitErr, emitSig, errors, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    toggleGoLive();
    expect(useVoice.getState().live).toBe(true);
    emitErr({ op: "err", e: "forbidden", s: "srv", c: "voice" });
    expect(useVoice.getState().live).toBe(false);
    expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
    expect(errors).toHaveLength(1);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("watches a live track without requesting a microphone", async () => {
    const { peers, mediaSent, sent, getUserMediaCalls } = install();
    watchLive({
      serverId: "srv",
      channelId: "voice",
      channelName: "Lounge",
    });
    expect(useVoice.getState().watching).toBe(true);
    expect(useVoice.getState().status).toBe("idle");
    await vi.waitFor(() => expect(peers.length).toBe(1));
    expect(getUserMediaCalls()).toBe(0);
    expect(peers[0]?.tracks).toBeGreaterThanOrEqual(2);
    expect(sent.some((frame) => frame.op === "sig" && frame.t === "j")).toBe(
      false,
    );
    expect(mediaSent.some((frame) => frame.op === "j")).toBe(true);
    const stream = fakeVideoStream("u-bob:l");
    peers[0]?.ontrack?.({
      track: stream.getVideoTracks()[0]!,
      streams: [stream],
    });
    expect(useVoice.getState().watchStream).toBe(stream);
    expect(useVoice.getState().remote["u-bob"]?.l).toBe(stream);
    const late = fakeVideoStream("u-cara:l-8801");
    peers[0]?.ontrack?.({
      track: late.getVideoTracks()[0]!,
      streams: [late],
    });
    expect(useVoice.getState().watchStream).toBe(late);
    expect(useVoice.getState().remote["u-cara"]?.l).toBe(late);
    const stray = fakeVideoStream("chrome-msid");
    peers[0]?.ontrack?.({
      track: stray.getVideoTracks()[0]!,
      streams: [stray],
    });
    expect(useVoice.getState().watchStream).toBe(late);
    stopWatching();
    expect(useVoice.getState().watching).toBe(false);
  });

  it("uses an untagged watch video only until a parsed live track arrives", async () => {
    const { peers } = install();
    watchLive({
      serverId: "srv",
      channelId: "voice",
      channelName: "Lounge",
    });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    const stray = fakeVideoStream("chrome-msid");
    peers[0]?.ontrack?.({
      track: stray.getVideoTracks()[0]!,
      streams: [stray],
    });
    expect(useVoice.getState().watchStream).toBe(stray);
    const live = fakeVideoStream("u-bob:l");
    peers[0]?.ontrack?.({
      track: live.getVideoTracks()[0]!,
      streams: [live],
    });
    expect(useVoice.getState().watchStream).toBe(live);
  });

  it("parses SFU stream ids for a live track", () => {
    expect(parseRemoteStreamId("u-bob:l")).toEqual({
      userId: "u-bob",
      k: "l",
    });
  });

  it("applies the selected Opus bitrate on the sender encodings", async () => {
    useMediaSettings.getState().patch({ quality: "high" });
    const { peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.senders.length).toBeGreaterThan(0));
    await vi.waitFor(() =>
      expect(
        peers[0]?.senders[0]?.getParameters?.().encodings[0]?.maxBitrate,
      ).toBe(128_000),
    );
    expect(peers[0]?.senders[0]?.getParameters?.().transactionId).toBe("tx-1");
  });

  it("captures with AEC off when the user turned it off", async () => {
    useMediaSettings.getState().patch({ echoCancellation: false });
    const { lastUserMedia, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    expect(lastUserMedia()).toEqual({
      audio: {
        echoCancellation: false,
        noiseSuppression: true,
        autoGainControl: true,
        channelCount: 1,
      },
      video: false,
    });
  });

  it("replaces the mic track when the input device changes, without leaving", async () => {
    const { peers, getUserMediaCalls } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    const first = peers[0]?.audio;
    expect(getUserMediaCalls()).toBe(1);
    useMediaSettings.getState().patch({ audioInputId: "mic-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    expect(useVoice.getState().status).toBe("joined");
    expect(peers[0]?.audio).toBeTruthy();
    expect(peers[0]?.audio).not.toBe(first);
  });

  it("keeps mute/deafen consistent when output volume changes", async () => {
    const { peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleMute();
    expect(peers[0]?.audio?.enabled).toBe(false);
    useMediaSettings.getState().patch({ outputVolume: 0.2 });
    expect(useVoice.getState().muted).toBe(true);
    expect(peers[0]?.audio?.enabled).toBe(false);
    toggleMute();
    expect(peers[0]?.audio?.enabled).toBe(true);
  });

  it("does not recapture the mic when only input gain changes", async () => {
    const { peers, getUserMediaCalls } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    expect(getUserMediaCalls()).toBe(1);
    useMediaSettings.getState().patch({ inputGain: 0.4 });
    useMediaSettings.getState().patch({ inputGain: 1.6 });
    await Promise.resolve();
    expect(getUserMediaCalls()).toBe(1);
    expect(useVoice.getState().status).toBe("joined");
    useMediaSettings.getState().patch({ noiseSuppression: false });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
  });

  it("sets GainNode.value in place and tears the insert down at identity", async () => {
    class FakeGain {
      gain = { value: 1 };
      connect(): void {}
      disconnect(): void {}
    }
    class FakeCtx {
      state: AudioContextState = "running";
      gain = new FakeGain();
      resume = async (): Promise<void> => {
        this.state = "running";
      };
      close = async (): Promise<void> => {
        this.state = "closed";
      };
      createMediaStreamSource(): { connect(): void } {
        return { connect() {} };
      }
      createGain(): FakeGain {
        return this.gain;
      }
      createMediaStreamDestination(): { stream: MediaStream } {
        return { stream: fakeStream() };
      }
    }
    const created: FakeCtx[] = [];
    const Prev = (globalThis as unknown as { AudioContext?: unknown })
      .AudioContext;
    (globalThis as unknown as { AudioContext: unknown }).AudioContext =
      class extends FakeCtx {
        constructor() {
          super();
          created.push(this);
        }
      };
    try {
      const { peers, getUserMediaCalls } = install();
      joinVoice({
        serverId: "srv",
        channelId: "voice",
        channelName: "Lounge",
      });
      await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
      expect(created).toHaveLength(0);
      useMediaSettings.getState().patch({ inputGain: 0.5 });
      await vi.waitFor(() => expect(created).toHaveLength(1));
      expect(created[0]?.gain.gain.value).toBe(0.5);
      expect(getUserMediaCalls()).toBe(1);
      const boosted = peers[0]?.audio;
      useMediaSettings.getState().patch({ inputGain: 1.5 });
      await vi.waitFor(() => expect(created[0]?.gain.gain.value).toBe(1.5));
      expect(created).toHaveLength(1);
      expect(created[0]?.state).toBe("running");
      expect(getUserMediaCalls()).toBe(1);
      expect(peers[0]?.audio).toBe(boosted);
      useMediaSettings.getState().patch({ inputGain: 1 });
      await vi.waitFor(() => expect(created[0]?.state).toBe("closed"));
      expect(getUserMediaCalls()).toBe(1);
      expect(peers[0]?.audio).not.toBe(boosted);
    } finally {
      (globalThis as unknown as { AudioContext?: unknown }).AudioContext = Prev;
    }
  });

  it("deafens Go Live watch audio with the same volume as the room mix", async () => {
    const clips: FakeHtmlAudio[] = [];
    class FakeHtmlAudio {
      autoplay = false;
      muted = false;
      volume = 1;
      srcObject: MediaStream | null = null;
      constructor() {
        clips.push(this);
      }
      setAttribute(): void {}
      play(): Promise<void> {
        return Promise.resolve();
      }
    }
    const Prev = globalThis.Audio;
    (globalThis as unknown as { Audio: typeof FakeHtmlAudio }).Audio =
      FakeHtmlAudio;
    try {
      const { peers } = install();
      joinVoice({
        serverId: "srv",
        channelId: "voice",
        channelName: "Lounge",
      });
      await vi.waitFor(() => expect(peers.length).toBe(1));
      watchLive({
        serverId: "srv",
        channelId: "stage",
        channelName: "Stage",
      });
      await vi.waitFor(() => expect(peers.length).toBe(2));
      const live = fakeVideoStream("u-bob:l", true);
      peers[1]?.ontrack?.({
        track: live.getAudioTracks()[0]!,
        streams: [live],
      });
      const watch = clips.find((el) => el.srcObject === live);
      expect(watch).toBeTruthy();
      expect(watch?.muted).toBe(false);
      expect(watch?.volume).toBe(1);
      toggleDeafen();
      expect(watch?.muted).toBe(true);
      expect(watch?.volume).toBe(0);
      useMediaSettings.getState().patch({ outputVolume: 0.25 });
      expect(watch?.muted).toBe(true);
      expect(watch?.volume).toBe(0);
      toggleDeafen();
      expect(watch?.muted).toBe(false);
      expect(watch?.volume).toBe(0.25);
    } finally {
      globalThis.Audio = Prev;
    }
  });
});
