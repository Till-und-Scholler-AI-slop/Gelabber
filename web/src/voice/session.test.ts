import * as callSounds from "./callSounds.ts";
import { useAudioProcessing } from "./audioProcessing.ts";
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
  resetVoiceForTests,
  toggleCamera,
  toggleDeafen,
  toggleGoLive,
  toggleMute,
  toggleShare,
  toggleSourceWatch,
  watchLive,
  stopWatching,
  retryPlayback,
  useVoice,
  type RtpSender,
} from "./session.ts";
import type {
  MediaConnection,
  MediaConnectionOptions,
  MediaPublication,
  MediaSender,
  ReceivedSource,
} from "./mediasoupConnection.ts";
import type { TrackKind } from "../ws/protocol.ts";
import { resetSessionForTests, useSession } from "../auth/session.ts";
import { randomUuid } from "../lib/uuid.ts";
import {
  buildDiagnosticExport,
  diagnosticsPolling,
  useVoiceDiagnostics,
} from "./diagnostics.ts";
import { setNativeBridgeForTests } from "./native/bridge.ts";
import { resetVoiceRoster, useVoiceRoster, voiceOf } from "./roster.ts";
import {
  allocateVideoBitrates,
  resetMediaSettingsForTests,
  sourceAudioChoice,
  useMediaSettings,
  videoConstraintsFor,
} from "./settings.ts";

// Session tests cover event decisions; native playback is exercised separately.
vi.mock("./callSounds.ts", () => ({
  playCallSound: vi.fn(),
  unlockCallSounds: vi.fn(),
  stopCallSounds: vi.fn(),
  setCallSoundsDeafened: vi.fn(),
}));

type MutableSender = Omit<MediaSender, "track"> & {
  track: MediaStreamTrack | null;
};
class FakeConnection implements MediaConnection {
  closed = false;
  started = false;
  tracks = 0;
  audio: MediaStreamTrack | null = null;
  audioEnabledWhenAdded: boolean[] = [];
  senderRows: MutableSender[] = [];
  publicationInputs: MediaPublication[] = [];
  private codecSignatures = new Map<TrackKind, string>();
  private receivedRows = new Map<string, ReceivedSource>();
  iceServers: IceServer[];
  states = { send: "new", recv: "new" };
  restartCalls: string[] = [];
  getStats = async (): Promise<import("./diagnostics.ts").StatsEntry[]> => [];
  constructor(
    readonly options: MediaConnectionOptions,
    readonly holdReplace?: Promise<void>,
    readonly holdPublish?: Promise<void>,
  ) {
    this.iceServers = options.iceServers;
  }
  async start(): Promise<void> {
    this.started = true;
  }
  async publish(input: MediaPublication): Promise<MediaSender> {
    if (this.closed) throw new Error("closed");
    this.publicationInputs.push(input);
    if (this.holdPublish && input.track.kind === "video")
      await this.holdPublish;
    const existing = this.sender(input.kind);
    const signature = JSON.stringify(this.options.codecOptions(input.kind));
    if (existing && this.codecSignatures.get(input.kind) === signature) {
      await existing.replaceTrack(input.track);
      return existing;
    }
    const result = await this.options.request(
      "produce",
      {
        k: input.kind,
        epoch: input.epoch,
        rtp: { codecs: [], encodings: [{}] },
        parent: input.parent,
        lc: input.lc,
        paused: input.paused,
        expectedOldProducerId: existing?.producerId,
      },
      input.deadlineEpochMs,
    );
    if (this.closed) throw new Error("closed");
    this.tracks++;
    if (input.track.kind === "audio")
      this.audioEnabledWhenAdded.push(input.track.enabled);
    let params: import("./mediasoupConnection.ts").RtpSenderParameters = {
      encodings: input.track.kind === "video" ? [{}, {}] : [{}],
      transactionId: `tx-${this.tracks}`,
    };
    const sender: MutableSender = {
      sourceKind: input.kind,
      producerId: result.producerId,
      epoch: input.epoch,
      track: input.track,
      rtpSender: undefined,
      replaceTrack: async (track) => {
        if (this.holdReplace) await this.holdReplace;
        sender.track = track;
        if (input.kind === "a") this.audio = track;
      },
      getParameters: () => ({
        ...params,
        encodings: params.encodings.map((p) => ({ ...p })),
      }),
      setParameters: async (next) => {
        if (next.transactionId !== params.transactionId)
          throw new Error("InvalidModificationError");
        params = { ...next, encodings: next.encodings.map((p) => ({ ...p })) };
      },
    };
    this.senderRows = this.senderRows.filter((held) => held !== existing);
    this.senderRows.push(sender);
    this.codecSignatures.set(input.kind, signature);
    if (input.kind === "a") this.audio = input.track;
    return sender;
  }
  sender(kind: TrackKind): MutableSender | undefined {
    return this.senderRows.find((s) => s.sourceKind === kind);
  }
  senders(): MutableSender[] {
    return this.senderRows;
  }
  consumers(): ReceivedSource[] {
    return [...this.receivedRows.values()];
  }
  receive(
    owner: string,
    kind: TrackKind,
    stream: MediaStream,
    consumerId = `consumer-${owner}-${kind}`,
    generation = "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
  ): ReceivedSource {
    const track = stream.getTracks()[0]!;
    const source = {
      consumerId,
      producerId: `producer-${owner}-${kind}`,
      owner,
      k: kind,
      epoch: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
      generation,
      parent:
        kind === "sa"
          ? `producer-${owner}-s`
          : kind === "la"
            ? `producer-${owner}-l`
            : undefined,
      kind: track.kind,
      rtpParameters: { codecs: [], encodings: [{ ssrc: 17 }] },
      paused: false,
      track,
      stream,
      consumer: { close: vi.fn(), pause: vi.fn(), resume: vi.fn() },
      layers: { spatial: null, temporal: null },
    } as unknown as ReceivedSource;
    this.receivedRows.set(consumerId, source);
    this.options.onConsumer(source);
    return source;
  }
  removeReceived(source: ReceivedSource): void {
    if (this.receivedRows.get(source.consumerId) === source)
      this.receivedRows.delete(source.consumerId);
    this.options.onConsumerClosed(source);
  }
  async closeSource(
    kind: TrackKind,
    expectedProducerId?: string,
  ): Promise<void> {
    const sender = this.sender(kind);
    if (
      sender &&
      (expectedProducerId === undefined ||
        sender.producerId === expectedProducerId)
    ) {
      this.senderRows = this.senderRows.filter((s) => s !== sender);
      await this.options.request("closeProducer", {
        producerId: sender.producerId,
      });
    }
  }
  async setSourcePaused(kind: TrackKind, paused: boolean): Promise<void> {
    const sender = this.sender(kind);
    if (sender)
      await this.options.request(paused ? "pauseProducer" : "resumeProducer", {
        producerId: sender.producerId,
      });
  }
  transportState(direction: "send" | "recv"): string {
    return this.states[direction];
  }
  async restartIce(direction: "send" | "recv"): Promise<void> {
    this.restartCalls.push(direction);
    await this.options.request("restartIce", { transportId: direction });
  }
  setTransportState(direction: "send" | "recv", state: string): void {
    this.states[direction] = state;
    this.options.onTransportState(direction, state);
  }
  handleEvent(): void {}
  close(): void {
    this.closed = true;
    this.senderRows = [];
    for (const source of [...this.receivedRows.values()])
      this.removeReceived(source);
  }
}

let trackSeq = 0;

type FakeTrack = MediaStreamTrack & { stopped: boolean };

function fakeTrack(kind: "audio" | "video", id?: string): FakeTrack {
  trackSeq += 1;
  const listeners = new Map<string, Set<EventListenerOrEventListenerObject>>();
  const track = {
    kind,
    get readyState() {
      return track.stopped ? "ended" : "live";
    },
    enabled: true,
    id: id ?? `${kind}-${trackSeq}`,
    stopped: false,
    stop() {
      track.stopped = true;
    },
    addEventListener(
      type: string,
      listener: EventListenerOrEventListenerObject,
    ) {
      const held = listeners.get(type) ?? new Set();
      held.add(listener);
      listeners.set(type, held);
    },
    removeEventListener(
      type: string,
      listener: EventListenerOrEventListenerObject,
    ) {
      listeners.get(type)?.delete(listener);
    },
    dispatchEvent(event: Event) {
      if (event.type === "ended") track.stopped = true;
      for (const listener of listeners.get(event.type) ?? []) {
        if (typeof listener === "function") listener(event);
        else listener.handleEvent(event);
      }
      return true;
    },
  };
  return track as unknown as FakeTrack;
}

function fakeStream(id = "mic"): MediaStream {
  const track = fakeTrack("audio", `${id}-a`);
  return {
    id,
    getTracks: () => [track],
    getAudioTracks: () => [track],
    getVideoTracks: () => [],
  } as unknown as MediaStream;
}

function fakeVideoStream(id: string, withAudio = false): MediaStream {
  const video = fakeTrack("video", `${id}-v`);
  const audio = fakeTrack("audio", `${id}-a`);
  const tracks = withAudio ? [video, audio] : [video];
  return {
    id,
    getTracks: () => tracks,
    getAudioTracks: () => (withAudio ? [audio] : []),
    getVideoTracks: () => [video],
  } as unknown as MediaStream;
}

function trackStopped(track: MediaStreamTrack | null | undefined): boolean {
  return Boolean(track && (track as FakeTrack).stopped);
}

function streamStopped(stream: MediaStream | null | undefined): boolean {
  return Boolean(stream?.getTracks().every((track) => trackStopped(track)));
}

function deferred(): { promise: Promise<void>; resolve: () => void } {
  let resolve!: () => void;
  const promise = new Promise<void>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

/** Apply this sender's next parameters, then pause before the caller continues. */
function holdNextSetParameters(sender: RtpSender): {
  entered: Promise<void>;
  release: () => void;
} {
  const original = sender.setParameters?.bind(sender);
  const gate = deferred();
  let enteredResolve!: () => void;
  const entered = new Promise<void>((resolve) => {
    enteredResolve = resolve;
  });
  let used = false;
  sender.setParameters = async (params) => {
    await original?.(params);
    if (used) return;
    used = true;
    enteredResolve();
    await gate.promise;
  };
  return { entered, release: gate.resolve };
}

function videoBitrates(
  peer: MediaConnection | undefined,
): Array<number | undefined> {
  return (peer?.senders() ?? [])
    .filter((sender) => sender.track?.kind === "video")
    .map((sender) => {
      const encodings = sender.getParameters().encodings;
      return encodings.every((e) => e.maxBitrate !== undefined)
        ? encodings.reduce((sum, e) => sum + e.maxBitrate!, 0)
        : undefined;
    });
}

/** Go Live claim traffic on the chat socket: "p" claims, "u" releases. */
function liveClaimFrames(sent: ClientFrame[]): string[] {
  return sent.flatMap((frame) =>
    frame.op === "sig" && "k" in frame && frame.k === "l" ? [frame.t] : [],
  );
}

function install(opts?: {
  media?: boolean;
  display?: boolean;
  userId?: string;
  ticketFail?: boolean;
  ticketError?: (index: number) => Error | undefined;
  gateTicket?: (index: number) => Promise<void> | void;
  iceServersFor?: (index: number) => IceServer[];
  holdMedia?: Promise<void>;
  holdDisplay?: Promise<void>;
  /** Per getDisplayMedia call index (0-based). An open picker. */
  gateDisplay?: (callIndex: number) => Promise<void> | void;
  /** The desktop app: the session's own display capture, through the bridge. */
  nativeDisplay?: boolean;
  /** Per getUserMedia call index (0-based). Controlled promise resolution. */
  gateMedia?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => Promise<void> | void;
  /** Optional stream factory after the gate opens. */
  mediaStreamFor?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => MediaStream;
  failMedia?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => boolean;
  /** Thrown instead of a generic denial. OverconstrainedError is retried. */
  mediaError?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => Error | undefined;
  displayStreamFor?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => MediaStream;
  displayError?: (
    callIndex: number,
    constraints: MediaStreamConstraints,
  ) => Error | undefined;
  holdReplaceTrack?: Promise<void>;
  holdPublish?: Promise<void>;
  produceError?: (kind: TrackKind) => Error | undefined;
  /** The server's answer to a "produce" waits for this. */
  gateProduce?: (kind: TrackKind) => Promise<void> | undefined;
  /** Hold the versioned Join response while rejection/cancellation is tested. */
  holdJoin?: boolean;
  holdLiveClaim?: boolean;
  mediaVersion?: number;
}) {
  useVoiceRoster.setState({
    live: {
      ...useVoiceRoster.getState().live,
      srv: { ...useVoiceRoster.getState().live.srv, stage: "u-bob" },
      "srv-a": { stage: "u-bob" },
    },
  });
  const sent: ClientFrame[] = [];
  const mediaSent: MediaClientFrame[] = [];
  let onSig: ((event: SigEvent) => void) | undefined;
  let onErr: ((err: ErrFrame) => void) | undefined;
  let onReady: (() => void) | undefined;
  let onMedia: ((frame: MediaServerFrame) => void) | undefined;
  let lingering: ((frame: MediaServerFrame) => void) | undefined;
  const mediaSockets: MediaSocket[] = [];
  const peers: FakeConnection[] = [];
  const errors: unknown[] = [];
  const streams: MediaStream[] = [];
  let getUserMediaCalls = 0;
  let getDisplayMediaCalls = 0;
  let ticketCalls = 0;
  let lastUserMedia: MediaStreamConstraints | undefined;
  let lastDisplayMedia: MediaStreamConstraints | undefined;

  configureVoice({
    userId: () => opts?.userId ?? "u-self",
    gateway: {
      send: (frame) => {
        sent.push(frame);
        if (
          frame.op === "sig" &&
          frame.t === "p" &&
          frame.k === "l" &&
          !opts?.holdLiveClaim
        ) {
          queueMicrotask(() =>
            onSig?.({
              op: "sig",
              t: "p",
              s: frame.s,
              c: frame.c,
              u: opts?.userId ?? "u-self",
              k: "l",
              lc: "00000000-0000-0000-0000-000000000001",
            } as SigEvent),
          );
        }
      },
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
    createMediaConnection: (options) => {
      const peer = new FakeConnection(
        options,
        opts?.holdReplaceTrack,
        opts?.holdPublish,
      );
      peers.push(peer);
      return peer;
    },
    getUserMedia: async (constraints) => {
      const callIndex = getUserMediaCalls;
      getUserMediaCalls += 1;
      lastUserMedia = constraints;
      const mediaError = opts?.mediaError?.(callIndex, constraints);
      if (mediaError) {
        if (opts?.gateMedia) await opts.gateMedia(callIndex, constraints);
        else if (opts?.holdMedia) await opts.holdMedia;
        throw mediaError;
      }
      if (opts?.media === false || opts?.failMedia?.(callIndex, constraints)) {
        if (opts?.gateMedia) await opts.gateMedia(callIndex, constraints);
        else if (opts?.holdMedia) await opts.holdMedia;
        throw new Error("denied");
      }
      const stream = opts?.mediaStreamFor
        ? opts.mediaStreamFor(callIndex, constraints)
        : constraints.video
          ? fakeVideoStream(`local-cam-${callIndex}`)
          : fakeStream(`mic-${callIndex}`);
      streams[callIndex] = stream;
      if (opts?.gateMedia) await opts.gateMedia(callIndex, constraints);
      else if (opts?.holdMedia) await opts.holdMedia;
      return stream;
    },
    getDisplayMedia: async (constraints) => {
      const callIndex = getDisplayMediaCalls;
      getDisplayMediaCalls += 1;
      lastDisplayMedia = constraints;
      if (opts?.gateDisplay) await opts.gateDisplay(callIndex);
      else if (opts?.holdDisplay) await opts.holdDisplay;
      const displayError = opts?.displayError?.(callIndex, constraints);
      if (displayError) throw displayError;
      if (opts?.display === false) throw new Error("denied");
      return (
        opts?.displayStreamFor?.(callIndex, constraints) ??
        fakeVideoStream("local-scr", true)
      );
    },
    fetchTicket: async () => {
      const index = ticketCalls++;
      if (opts?.gateTicket) await opts.gateTicket(index);
      const error = opts?.ticketError?.(index);
      if (error) throw error;
      if (opts?.ticketFail) throw new Error("ticket");
      return {
        ticket: "abcdefghjkmn",
        media_path: "/media/ws",
        ice_servers: opts?.iceServersFor?.(index) ?? [
          { urls: ["stun:127.0.0.1:3478"] },
        ],
      };
    },
    openMedia: () => {
      const closeHandlers = new Set<() => void>();
      let alive = true;
      const socket: MediaSocket = {
        send: (frame) => {
          mediaSent.push(frame);
          if (frame.op === "j" && opts?.holdJoin) return;
          const error =
            frame.op === "produce" ? opts?.produceError?.(frame.k) : undefined;
          if (error) {
            queueMicrotask(() =>
              (onMedia ?? lingering)?.({
                op: "err",
                id: frame.id,
                e: error.message,
              }),
            );
            return;
          }
          const answer = () => {
            if (!alive) return;
            (onMedia ?? lingering)?.({
              op: "result",
              id: frame.id,
              data:
                frame.op === "j"
                  ? {
                      c: "voice",
                      u: opts?.userId ?? "u-self",
                      v: opts?.mediaVersion ?? 4,
                      generation: randomUuid(),
                      routerRtpCapabilities: { codecs: [] },
                    }
                  : frame.op === "produce"
                    ? { producerId: "producer-" + frame.id }
                    : frame.op === "restartIce"
                      ? {
                          iceParameters: {
                            usernameFragment: "x",
                            password: "y",
                          },
                        }
                      : {},
            });
          };
          const gate =
            frame.op === "produce" ? opts?.gateProduce?.(frame.k) : undefined;
          if (gate) void gate.then(answer);
          else queueMicrotask(answer);
        },
        close() {
          if (!alive) return;
          alive = false;
          for (const handler of [...closeHandlers]) handler();
        },
        onFrame(handler) {
          onMedia = handler;
          lingering = handler;
          return () => {
            if (onMedia === handler) onMedia = undefined;
          };
        },
        onClose(handler) {
          if (!alive) {
            handler();
            return () => {};
          }
          closeHandlers.add(handler);
          return () => {
            closeHandlers.delete(handler);
          };
        },
      };
      mediaSockets.push(socket);
      return socket;
    },
    onError: (error) => errors.push(error),
    ...(opts?.nativeDisplay ? { getDisplayMedia: undefined } : {}),
  });

  return {
    sent,
    mediaSent,
    peers,
    errors,
    streams,
    emitSig: (event: SigEvent & { lc?: string }) => onSig?.(event),
    emitErr: (err: ErrFrame) => onErr?.(err),
    emitReady: () => onReady?.(),
    emitMedia: (frame: MediaServerFrame) => (onMedia ?? lingering)?.(frame),
    closeMedia: () => mediaSockets.at(-1)?.close(),
    getUserMediaCalls: () => getUserMediaCalls,
    getDisplayMediaCalls: () => getDisplayMediaCalls,
    ticketCalls: () => ticketCalls,
    lastUserMedia: () => lastUserMedia,
    lastDisplayMedia: () => lastDisplayMedia,
  };
}

describe("voice session", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    trackSeq = 0;
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("sounds only for new room arrivals and known departures, not snapshots or reconnect replays", () => {
    const { emitSig, emitReady } = install();
    const sound = vi
      .spyOn(callSounds, "playCallSound")
      .mockImplementation(() => {});
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    expect(sound).not.toHaveBeenCalled();
    const event = {
      op: "sig" as const,
      t: "j" as const,
      s: "srv",
      c: "voice",
      u: "peer",
    };
    emitSig({ ...event, replay: true });
    emitSig({ ...event, u: "u-self" });
    emitSig({ ...event, u: "u-self" });
    expect(sound.mock.calls).toEqual([["join"]]);
    sound.mockClear();
    emitSig(event); // already present
    emitSig({ ...event, c: "other", u: "other" });
    emitSig({ ...event, t: "r", snap: [{ u: "peer", c: "voice" }] });
    emitSig({ ...event, t: "l", u: "unknown" });
    expect(sound).not.toHaveBeenCalled();
    emitSig({ ...event, u: "new" });
    emitSig({ ...event, u: "new" });
    emitSig({ ...event, t: "l", u: "new" });
    emitSig({ ...event, t: "l", u: "new" });
    expect(sound.mock.calls).toEqual([["join"], ["leave"]]);
    sound.mockClear();
    emitReady();
    emitSig({ ...event, u: "u-self" });
    emitSig({ ...event, replay: true });
    expect(sound).not.toHaveBeenCalled();
  });

  it("plays own control feedback once and no leave cue for an unacknowledged join", () => {
    install();
    const sound = vi
      .spyOn(callSounds, "playCallSound")
      .mockImplementation(() => {});
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    toggleMute();
    toggleMute();
    toggleDeafen();
    toggleDeafen();
    expect(sound.mock.calls).toEqual([
      ["mute"],
      ["unmute"],
      ["deafen"],
      ["undeafen"],
    ]);
    sound.mockClear();
    leaveVoice();
    expect(sound).not.toHaveBeenCalled();
  });

  it("join click sets local state before any ICE work", () => {
    const { sent, mediaSent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });

    const state = useVoice.getState();
    expect(state.status).toBe("joined");
    expect(state.channelId).toBe("voice");
    expect(state.participants["u-self"]).toEqual({ pubs: [] });
    expect(sent[0]).toEqual({ op: "sig", t: "j", s: "srv", c: "voice" });
    expect(
      sent.filter((frame) => frame.op === "sig").map((frame) => frame.t),
    ).toEqual(["j"]);
    expect(mediaSent.some((frame) => frame.op === "produce")).toBe(false);
  });

  it("publishes via correlated v4 media WS and keeps presence on chat WS", async () => {
    const { sent, mediaSent, peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => {
      expect(mediaSent.some((frame) => frame.op === "produce")).toBe(true);
    });
    expect(peers).toHaveLength(1);
    expect(peers[0]?.tracks).toBe(1);
    expect(peers[0]?.iceServers[0]?.urls).toEqual(["stun:127.0.0.1:3478"]);
    expect(mediaSent.map((frame) => frame.op)).toContain("produce");
    expect(
      mediaSent.every((frame) => Number.isInteger(frame.id) && frame.id > 0),
    ).toBe(true);
    expect(mediaSent[0]).toEqual({ op: "j", id: 1, tk: "abcdefghjkmn", v: 4 });
    expect(
      sent.filter((frame) => frame.op === "sig").map((frame) => frame.t),
    ).toEqual(["j", "p"]);
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
        sampleRate: { ideal: 48_000 },
      },
      video: false,
    });
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

  it("leaves an open media peer alone when the chat socket reconnects", async () => {
    const { emitSig, emitReady, peers, mediaSent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "j")).toBe(true),
    );
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    emitReady();
    expect(peers).toHaveLength(1);
    expect(mediaSent.filter((frame) => frame.op === "j")).toHaveLength(1);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("rebuilds the media peer after the transport closes", async () => {
    const { emitReady, mediaSent, peers, closeMedia } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(mediaSent.filter((frame) => frame.op === "j")).toHaveLength(1),
    );
    closeMedia();
    await vi.waitFor(() =>
      expect(mediaSent.filter((frame) => frame.op === "j")).toHaveLength(2),
    );
    expect(peers).toHaveLength(2);
    emitReady();
    expect(mediaSent.filter((frame) => frame.op === "j")).toHaveLength(2);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("re-announces active tracks after the gateway join is confirmed", async () => {
    const { sent, peers, emitReady, emitSig, emitErr, getDisplayMediaCalls } =
      install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    const displays = getDisplayMediaCalls();
    emitReady();
    const afterReady = sent.length;
    expect(sent.at(-1)).toEqual({ op: "sig", t: "j", s: "srv", c: "voice" });
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    expect(sent.slice(afterReady)).toEqual([
      { op: "sig", t: "p", s: "srv", c: "voice", k: "a" },
      { op: "sig", t: "p", s: "srv", c: "voice", k: "l" },
    ]);
    expect(getDisplayMediaCalls()).toBe(displays);
    expect(useVoice.getState().live).toBe(true);
    emitErr({ op: "err", e: "bad_request", s: "srv", c: "voice" });
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
    expect(getDisplayMediaCalls()).toBe(displays);
  });

  it("unpublishes announced tracks when the media attempt is unavailable", async () => {
    const { emitMedia, peers, sent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    await vi.waitFor(() =>
      expect(
        sent.some(
          (frame) => frame.op === "sig" && frame.t === "p" && frame.k === "a",
        ),
      ).toBe(true),
    );
    toggleGoLive();
    await vi.waitFor(() =>
      expect(
        sent.some(
          (frame) => frame.op === "sig" && frame.t === "p" && frame.k === "l",
        ),
      ).toBe(true),
    );
    expect(useVoice.getState().live).toBe(true);
    expect(useVoiceRoster.getState().live.srv?.voice).toBe("u-self");
    const before = sent.length;
    emitMedia({ op: "err", e: "unavailable" });
    expect(useVoice.getState().status).toBe("joined");
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().camera).toBe(false);
    expect(useVoice.getState().sharing).toBe(false);
    expect(useVoice.getState().localLive).toBeNull();
    expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
    expect(sent.slice(before).filter((frame) => frame.op === "sig")).toEqual([
      { op: "sig", t: "u", s: "srv", c: "voice", k: "a" },
      { op: "sig", t: "u", s: "srv", c: "voice", k: "l" },
    ]);
    expect(sent.some((frame) => frame.op === "sig" && frame.t === "l")).toBe(
      false,
    );
  });

  it("keeps unrelated Live capture when an unscoped publication denial arrives", async () => {
    const { emitMedia, errors, mediaSent } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(mediaSent.some((frame) => frame.op === "j")).toBe(true),
    );
    toggleGoLive();
    expect(useVoice.getState().live).toBe(true);
    emitMedia({ op: "err", e: "forbidden" });
    expect(useVoice.getState().status).toBe("joined");
    expect(useVoice.getState().live).toBe(true);
    expect(errors.map((error) => (error as Error).message)).toContain(
      "Dafür fehlt dir die Berechtigung.",
    );
  });

  it("retains capture across Live claim withdrawal and ignores the old claim after recovery", async () => {
    const { emitMedia, emitSig, emitReady, peers, getDisplayMediaCalls } =
      install({ holdLiveClaim: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    const stream = useVoice.getState().localLive!;
    const old = "00000000-0000-0000-0000-000000000001";
    const next = "00000000-0000-0000-0000-000000000002";
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: old,
    });
    await vi.waitFor(() =>
      expect(
        peers[0]?.senders().some((s) => s.track === stream.getVideoTracks()[0]),
      ).toBe(true),
    );
    emitMedia({ op: "err", e: "forbidden", lc: old });
    expect(useVoice.getState().live).toBe(true);
    expect(streamStopped(stream)).toBe(false);
    emitReady();
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: next,
    });
    emitMedia({ op: "err", e: "forbidden", lc: old });
    expect(useVoice.getState().localLive).toBe(stream);
    expect(streamStopped(stream)).toBe(false);
    expect(getDisplayMediaCalls()).toBe(1);
    expect(peers).toHaveLength(1);
  });

  it("bounds withdrawn Live claim recovery even across duplicate errors and Gateway rejoin", async () => {
    const { emitMedia, emitSig, emitReady, peers } = install({
      holdLiveClaim: true,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    const stream = useVoice.getState().localLive!;
    const nonce = "00000000-0000-0000-0000-000000000001";
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: nonce,
    });
    vi.useFakeTimers();
    emitMedia({ op: "err", e: "forbidden", lc: nonce });
    await vi.advanceTimersByTimeAsync(9000);
    expect(streamStopped(stream)).toBe(false);
    emitMedia({ op: "err", e: "forbidden", lc: nonce });
    emitReady();
    emitSig({ op: "sig", t: "j", s: "srv", c: "voice", u: "u-self" });
    await vi.advanceTimersByTimeAsync(1001);
    expect(useVoice.getState().live).toBe(false);
    expect(streamStopped(stream)).toBe(true);
    expect(useVoice.getState().status).toBe("joined");
    expect(trackStopped(peers[0]?.audio)).toBe(false);
  });

  it("cannot resume a withdrawn nonce", async () => {
    const { emitMedia, emitSig } = install({ holdLiveClaim: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(useVoice.getState().status).toBe("joined"));
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    const stream = useVoice.getState().localLive!;
    const nonce = "00000000-0000-0000-0000-000000000001";
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: nonce,
    });
    emitMedia({ op: "err", e: "forbidden", lc: nonce });
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: nonce,
    });
    expect(useVoice.getState().live).toBe(false);
    expect(streamStopped(stream)).toBe(true);
    emitMedia({ op: "err", e: "unauthorized" });
    expect(useVoice.getState().status).toBe("idle");
  });

  it("ends whole-peer authorization loss during pending Live recovery", async () => {
    const { emitMedia, emitSig, peers } = install({ holdLiveClaim: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    const stream = useVoice.getState().localLive!;
    const nonce = "00000000-0000-0000-0000-000000000001";
    emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: nonce,
    });
    emitMedia({ op: "err", e: "forbidden", lc: nonce });
    expect(streamStopped(stream)).toBe(false);
    emitMedia({ op: "err", e: "unauthorized" });
    expect(useVoice.getState().status).toBe("idle");
    expect(streamStopped(stream)).toBe(true);
    expect(peers[0]?.closed).toBe(true);
    expect(trackStopped(peers[0]?.audio)).toBe(true);
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

  it.each([
    { serverId: "srv", channelId: "other" },
    { serverId: "other-server", channelId: "voice" },
  ])(
    "preserves deliberate mute when switching to $serverId/$channelId",
    async (target) => {
      const f = install();
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
      await vi.waitFor(() => expect(f.peers[0]?.audio).toBeTruthy());
      toggleMute();
      const before = f.sent.length;
      joinVoice({ ...target, channelName: "New room" });
      expect(useVoice.getState().muted).toBe(true);
      expect(useVoice.getState().deafened).toBe(false);
      expect(f.sent.slice(before)).toEqual([
        { op: "sig", t: "l", s: "srv", c: "voice" },
        { op: "sig", t: "j", s: target.serverId, c: target.channelId },
        {
          op: "sig",
          t: "m",
          s: target.serverId,
          c: target.channelId,
          on: true,
        },
      ]);
      expect(
        voiceOf(useVoiceRoster.getState().byServer, target.serverId, "u-self")
          ?.muted,
      ).toBe(true);
      await vi.waitFor(() => expect(f.peers[1]?.audio).toBeTruthy());
      expect(f.peers[0]?.closed).toBe(true);
      expect(f.peers[1].audioEnabledWhenAdded).toEqual([false]);
      expect(f.peers[1]?.audio?.enabled).toBe(false);
    },
  );

  it.each([false, true])(
    "restores the pre-deafen mute choice (%s) after an explicit switch",
    async (initiallyMuted) => {
      const f = install();
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
      await vi.waitFor(() => expect(f.peers[0]?.audio).toBeTruthy());
      if (initiallyMuted) toggleMute();
      toggleDeafen();
      const before = f.sent.length;
      joinVoice({ serverId: "srv", channelId: "other", channelName: "Other" });
      expect(useVoice.getState()).toMatchObject({
        muted: true,
        deafened: true,
      });
      expect(f.sent.slice(before)).toEqual([
        { op: "sig", t: "l", s: "srv", c: "voice" },
        { op: "sig", t: "j", s: "srv", c: "other" },
        { op: "sig", t: "m", s: "srv", c: "other", on: true },
        { op: "sig", t: "d", s: "srv", c: "other", on: true },
      ]);
      await vi.waitFor(() => expect(f.peers[1]?.audio).toBeTruthy());
      expect(f.peers[1].audioEnabledWhenAdded).toEqual([false]);
      const beforeHearing = f.sent.length;
      toggleDeafen();
      expect(useVoice.getState()).toMatchObject({
        muted: initiallyMuted,
        deafened: false,
      });
      expect(f.peers[1]?.audio?.enabled).toBe(!initiallyMuted);
      expect(f.sent.slice(beforeHearing)).toEqual([
        ...(initiallyMuted
          ? []
          : [{ op: "sig", t: "m", s: "srv", c: "other", on: false }]),
        { op: "sig", t: "d", s: "srv", c: "other", on: false },
      ]);
    },
  );

  it("resynchronizes preserved flags on gateway reconnect", () => {
    const f = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    toggleMute();
    toggleDeafen();
    joinVoice({ serverId: "srv", channelId: "other", channelName: "Other" });
    const before = f.sent.length;
    f.emitReady();
    expect(f.sent.slice(before)).toEqual([
      { op: "sig", t: "j", s: "srv", c: "other" },
      { op: "sig", t: "m", s: "srv", c: "other", on: true },
      { op: "sig", t: "d", s: "srv", c: "other", on: true },
    ]);
    toggleDeafen();
    expect(useVoice.getState()).toMatchObject({ muted: true, deafened: false });
  });

  it("explicit unmute while deafened clears the old mute memory", () => {
    install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    toggleMute();
    toggleDeafen();
    joinVoice({ serverId: "srv", channelId: "other", channelName: "Other" });
    toggleMute();
    expect(useVoice.getState()).toMatchObject({
      muted: false,
      deafened: false,
    });
    toggleDeafen();
    toggleDeafen();
    expect(useVoice.getState()).toMatchObject({
      muted: false,
      deafened: false,
    });
  });

  it("starts a fresh session after leave with default mute and deafen", () => {
    install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    toggleMute();
    toggleDeafen();
    leaveVoice();
    joinVoice({ serverId: "srv", channelId: "other", channelName: "Other" });
    expect(useVoice.getState()).toMatchObject({
      muted: false,
      deafened: false,
    });
    toggleDeafen();
    toggleDeafen();
    expect(useVoice.getState()).toMatchObject({
      muted: false,
      deafened: false,
    });
  });

  it("disposes late old-room capture and keeps new-room capture muted", async () => {
    const gate = deferred();
    const f = install({
      gateMedia: (index) => (index === 0 ? gate.promise : undefined),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(f.getUserMediaCalls()).toBe(1));
    toggleMute();
    joinVoice({ serverId: "srv", channelId: "other", channelName: "Other" });
    await vi.waitFor(() => expect(f.peers[1]?.audio).toBeTruthy());
    expect(f.peers[1].audioEnabledWhenAdded).toEqual([false]);
    gate.resolve();
    await vi.waitFor(() => expect(streamStopped(f.streams[0])).toBe(true));
    expect(useVoice.getState()).toMatchObject({
      channelId: "other",
      muted: true,
    });
  });

  it("says so when the camera is missing or busy, and nothing after a refusal", async () => {
    let failure = Object.assign(new Error("no camera"), {
      name: "NotFoundError",
    });
    const { peers, errors } = install({
      mediaError: (_call, constraints) =>
        constraints.video ? failure : undefined,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    const attempt = async () => {
      toggleCamera();
      expect(useVoice.getState().camera).toBe(true);
      await vi.waitFor(() => expect(useVoice.getState().camera).toBe(false));
    };
    await attempt();
    expect(errors.map((error) => (error as Error).message)).toEqual([
      "Die Kamera ist nicht verfügbar.",
    ]);
    // What the desktop app's capture throws for a camera it cannot open.
    failure = Object.assign(new Error("Kamera nicht verfügbar: busy"), {
      name: "NotReadableError",
    });
    await attempt();
    expect(errors).toHaveLength(2);
    // The user said no, or closed the prompt.
    failure = Object.assign(new Error("denied"), { name: "NotAllowedError" });
    await attempt();
    expect(errors).toHaveLength(2);
    expect(useVoice.getState().localCamera).toBeNull();
  });

  it("shows a local camera preview before any publication", async () => {
    const publication = deferred();
    const { sent, mediaSent, peers } = install({
      holdPublish: publication.promise,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleCamera();
    expect(useVoice.getState().camera).toBe(true);
    expect(useVoice.getState().localCamera).toBeNull();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    expect(sent.some((frame) => "sdp" in frame && frame.sdp)).toBe(false);
    expect(
      sent.filter((frame) => frame.op === "sig" && frame.t === "p"),
    ).toEqual(
      expect.arrayContaining([
        { op: "sig", t: "p", s: "srv", c: "voice", k: "a" },
      ]),
    );
    expect(
      mediaSent.some((frame) => frame.op === "produce" && frame.k === "v"),
    ).toBe(false);
    publication.resolve();

    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "produce" && frame.k === "v"),
      ).toBe(true),
    );
  });

  it("starts screen-share without putting SDP on the chat socket", async () => {
    const publication = deferred();
    const { sent, mediaSent, peers } = install({
      holdPublish: publication.promise,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleShare();
    expect(useVoice.getState().sharing).toBe(true);
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    for (const frame of sent) {
      expect(frame).not.toHaveProperty("sdp");
      expect(frame).not.toHaveProperty("token");
    }
    expect(
      mediaSent.some((frame) => frame.op === "produce" && frame.k === "s"),
    ).toBe(false);
    publication.resolve();

    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "produce" && frame.k === "s"),
      ).toBe(true),
    );
    const videoSenders = peers[0]?.senderRows.filter(
      (sender) => sender.track?.kind === "video",
    );
    const audioSenders = peers[0]?.senderRows.filter(
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

  it("shows the Live badge immediately and publishes after display capture", async () => {
    const publication = deferred();
    const { sent, mediaSent, peers } = install({
      holdPublish: publication.promise,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    toggleGoLive();
    expect(useVoice.getState().live).toBe(true);
    expect(useVoice.getState().localLive).toBeNull();
    expect(useVoiceRoster.getState().live.srv?.voice).toBe("u-self");
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
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
    expect(
      mediaSent.some((frame) => frame.op === "produce" && frame.k === "l"),
    ).toBe(false);
    publication.resolve();

    await vi.waitFor(() =>
      expect(
        mediaSent.some((frame) => frame.op === "produce" && frame.k === "l"),
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

  it("bounds a missing Live acknowledgement and rejects a late one after Stop", async () => {
    const env = install({ holdLiveClaim: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    toggleGoLive();
    await vi.advanceTimersByTimeAsync(0);
    const stream = useVoice.getState().localLive!;
    expect(stream).toBeTruthy();
    await vi.advanceTimersByTimeAsync(10000);
    expect(useVoice.getState().live).toBe(false);
    expect(streamStopped(stream)).toBe(true);
    expect(env.errors).toHaveLength(1);
    env.emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: "00000000-0000-0000-0000-000000000042",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "l"),
    ).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
    leaveVoice();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("releases the Live claim when display capture is cancelled or fails", async () => {
    for (const error of [
      new DOMException("Freigabe abgebrochen", "NotAllowedError"),
      // A phone has no getDisplayMedia at all.
      new TypeError("navigator.mediaDevices.getDisplayMedia is not a function"),
    ]) {
      const env = install({ displayError: () => error });
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
      await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
      vi.useFakeTimers();
      toggleGoLive();
      expect(useVoice.getState().live).toBe(true);
      expect(liveClaimFrames(env.sent)).toEqual(["p"]);
      await vi.advanceTimersByTimeAsync(0);
      expect(useVoice.getState().live).toBe(false);
      expect(useVoice.getState().localLive).toBeNull();
      expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
      expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
      expect(env.sent.at(-1)).toEqual({
        op: "sig",
        t: "u",
        s: "srv",
        c: "voice",
        k: "l",
      });
      // No stale confirmation timer, no late toast, the seat stays.
      expect(vi.getTimerCount()).toBe(0);
      await vi.advanceTimersByTimeAsync(10000);
      expect(env.errors).toHaveLength(0);
      expect(useVoice.getState().status).toBe("joined");
      expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);

      // The channel is free again: the next attempt claims afresh.
      toggleGoLive();
      expect(liveClaimFrames(env.sent)).toEqual(["p", "u", "p"]);
      await vi.advanceTimersByTimeAsync(0);
      expect(liveClaimFrames(env.sent)).toEqual(["p", "u", "p", "u"]);

      vi.useRealTimers();
      resetVoiceForTests();
      resetVoiceRoster();
    }
  });

  it("releases the Live claim when the picker is cancelled after the claim was confirmed", async () => {
    const picker = deferred();
    const env = install({
      holdDisplay: picker.promise,
      displayError: () =>
        new DOMException("Freigabe abgebrochen", "NotAllowedError"),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    // The harness confirms the claim while the picker is still open.
    await vi.waitFor(() =>
      expect(useVoice.getState().participants["u-self"]?.pubs).toContain("l"),
    );
    expect(useVoice.getState().live).toBe(true);
    picker.resolve();
    await vi.waitFor(() => expect(useVoice.getState().live).toBe(false));
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("l");
    expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "l"),
    ).toBe(false);
    expect(env.errors).toHaveLength(0);
  });

  it("keeps a newer Live claim when an abandoned picker is cancelled late", async () => {
    const first = deferred();
    const env = install({
      gateDisplay: (index) => (index === 0 ? first.promise : undefined),
      displayError: (index) =>
        index === 0
          ? new DOMException("Freigabe abgebrochen", "NotAllowedError")
          : undefined,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    toggleGoLive();
    toggleGoLive();
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    toggleGoLive();
    await vi.waitFor(() =>
      expect(env.peers[0]!.sender("l")?.track).toBeTruthy(),
    );
    const stream = useVoice.getState().localLive!;
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u", "p"]);

    first.resolve();
    for (let i = 0; i < 30; i++) await Promise.resolve();
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u", "p"]);
    expect(useVoice.getState().live).toBe(true);
    expect(useVoice.getState().localLive).toBe(stream);
    expect(streamStopped(stream)).toBe(false);
    expect(useVoiceRoster.getState().live.srv?.voice).toBe("u-self");
    expect(env.peers[0]!.sender("l")?.track).toBe(stream.getVideoTracks()[0]);
    expect(env.errors).toHaveLength(0);
  });

  it("releases a Live claim the server has not confirmed yet", async () => {
    // A phone fails the capture before the claim's round trip is back.
    const env = install({
      holdLiveClaim: true,
      displayError: () =>
        new TypeError(
          "navigator.mediaDevices.getDisplayMedia is not a function",
        ),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    vi.useFakeTimers();
    toggleGoLive();
    expect(vi.getTimerCount()).toBe(1);
    await vi.advanceTimersByTimeAsync(0);
    expect(useVoice.getState().live).toBe(false);
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    // The confirmation deadline went with the claim.
    expect(vi.getTimerCount()).toBe(0);

    // Both echoes arrive late, in socket order.
    env.emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: "00000000-0000-0000-0000-000000000042",
    });
    expect(useVoice.getState().live).toBe(false);
    env.emitSig({
      op: "sig",
      t: "u",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
    });
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().localLive).toBeNull();
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("l");
    await vi.advanceTimersByTimeAsync(10000);
    expect(env.errors).toHaveLength(0);
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "l"),
    ).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("gives a capture back when its source ended before it could start", async () => {
    // "Stop sharing" while the stream profile is still being applied: the
    // picker has answered, and the track's only "ended" event is already gone.
    const ended = (id: string) => {
      const stream = fakeVideoStream(id, true);
      const video = stream.getVideoTracks()[0]!;
      video.applyConstraints = async () => {
        video.dispatchEvent(new Event("ended"));
        throw new DOMException("Track ended", "InvalidStateError");
      };
      return stream;
    };
    const captures = [ended("ended-live"), ended("ended-share")];
    // A camera that is unplugged while its prompt is open.
    const camera = fakeVideoStream("ended-cam");
    camera.getVideoTracks()[0]!.stop();
    const env = install({
      displayStreamFor: (index) => captures[index]!,
      mediaStreamFor: (index, constraints) =>
        constraints.video ? camera : fakeStream(`mic-${index}`),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());

    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().live).toBe(false));
    expect(useVoice.getState().localLive).toBeNull();
    expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("l");
    expect(streamStopped(captures[0])).toBe(true);

    toggleShare();
    await vi.waitFor(() => expect(useVoice.getState().sharing).toBe(false));
    expect(useVoice.getState().localScreen).toBeNull();
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("s");
    expect(streamStopped(captures[1])).toBe(true);

    toggleCamera();
    await vi.waitFor(() => expect(useVoice.getState().camera).toBe(false));
    expect(useVoice.getState().localCamera).toBeNull();
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("v");

    expect(env.getDisplayMediaCalls()).toBe(2);
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k !== "a"),
    ).toBe(false);
    expect(env.errors).toHaveLength(0);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("opens one picker when Go Live is clicked while the seat is still connecting", async () => {
    const ticket = deferred();
    const picker = deferred();
    const env = install({
      gateTicket: () => ticket.promise,
      gateDisplay: (index) => (index === 0 ? picker.promise : undefined),
      // A browser refuses a second picker that no click asked for.
      displayError: (index) =>
        index > 0
          ? new DOMException("Kein Klick", "InvalidStateError")
          : undefined,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    toggleGoLive();
    expect(env.getDisplayMediaCalls()).toBe(1);
    ticket.resolve();
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(useVoice.getState().live).toBe(true);
    expect(liveClaimFrames(env.sent)).toEqual(["p"]);

    // The source the user picks is the one that goes live.
    picker.resolve();
    await vi.waitFor(() =>
      expect(env.peers[0]!.sender("l")?.track).toBeTruthy(),
    );
    const stream = useVoice.getState().localLive!;
    expect(streamStopped(stream)).toBe(false);
    expect(env.peers[0]!.sender("l")?.track).toBe(stream.getVideoTracks()[0]);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(liveClaimFrames(env.sent)).toEqual(["p"]);
    expect(useVoiceRoster.getState().live.srv?.voice).toBe("u-self");
    expect(env.errors).toHaveLength(0);
  });

  it("captures camera and share once when both start while the seat is still connecting", async () => {
    const ticket = deferred();
    const prompt = deferred();
    const picker = deferred();
    const env = install({
      gateTicket: () => ticket.promise,
      gateMedia: (_index, constraints) =>
        constraints.video ? prompt.promise : undefined,
      gateDisplay: () => picker.promise,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    toggleCamera();
    toggleShare();
    ticket.resolve();
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    await new Promise((resolve) => setTimeout(resolve, 0));
    // Camera and microphone; the share picker only once.
    expect(env.getUserMediaCalls()).toBe(2);
    expect(env.getDisplayMediaCalls()).toBe(1);

    prompt.resolve();
    picker.resolve();
    await vi.waitFor(() => {
      expect(env.peers[0]!.sender("v")?.track).toBeTruthy();
      expect(env.peers[0]!.sender("s")?.track).toBeTruthy();
    });
    expect(streamStopped(useVoice.getState().localCamera)).toBe(false);
    expect(
      trackStopped(useVoice.getState().localScreen?.getVideoTracks()[0]),
    ).toBe(false);
    expect(env.getUserMediaCalls()).toBe(2);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(env.errors).toHaveLength(0);
  });

  it("does not reopen a picker cancelled while the seat finished connecting", async () => {
    const ticket = deferred();
    const picker = deferred();
    const publish = deferred();
    const env = install({
      gateTicket: () => ticket.promise,
      gateDisplay: () => picker.promise,
      displayError: () =>
        new DOMException("Freigabe abgebrochen", "NotAllowedError"),
      holdPublish: publish.promise,
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    toggleCamera();
    toggleGoLive();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    ticket.resolve();
    // The connect is past its last look at the buttons: the camera publishes.
    await vi.waitFor(() =>
      expect(
        env.peers[0]?.publicationInputs.some((input) => input.kind === "v"),
      ).toBe(true),
    );
    picker.resolve();
    await vi.waitFor(() => expect(useVoice.getState().live).toBe(false));
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);

    publish.resolve();
    await vi.waitFor(() =>
      expect(env.peers[0]!.sender("v")?.track).toBeTruthy(),
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().localLive).toBeNull();
    expect(env.errors).toHaveLength(0);
  });

  it("matches watch events by server and channel", async () => {
    const { emitSig, peers } = install();
    watchLive({
      serverId: "srv-a",
      channelId: "stage",
      channelName: "Stage",
    });
    await vi.waitFor(() => expect(peers.length).toBe(1));
    emitSig({
      op: "sig",
      t: "u",
      s: "srv-b",
      c: "stage",
      u: "u-bob",
      k: "l",
    });
    expect(useVoice.getState().watching).toBe(true);
    emitSig({
      op: "sig",
      t: "u",
      s: "srv-a",
      c: "stage",
      u: "u-bob",
      k: "l",
    });
    expect(useVoice.getState().watching).toBe(false);
  });

  it("applies the selected Opus bitrate on the sender encodings", async () => {
    useMediaSettings.getState().patch({ quality: "high", economyMode: true });
    const { peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() =>
      expect(peers[0]?.senderRows.length).toBeGreaterThan(0),
    );
    await vi.waitFor(() =>
      expect(
        peers[0]?.senderRows[0]?.getParameters?.().encodings[0]?.maxBitrate,
      ).toBe(128_000),
    );
    expect(peers[0]?.senderRows[0]?.getParameters?.().transactionId).toBe(
      "tx-1",
    );
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
        sampleRate: { ideal: 48_000 },
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
    await vi.waitFor(() => expect(peers[0]?.audio).not.toBe(first));
    expect(useVoice.getState().status).toBe("joined");
    expect(peers[0]?.audio).toBeTruthy();
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
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
  });

  it("sets GainNode.value in place, retaining the graph at identity until leave", async () => {
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
      await vi.waitFor(() => expect(created[0]?.gain.gain.value).toBe(1));
      expect(created[0]?.state).toBe("running");
      expect(getUserMediaCalls()).toBe(1);
      expect(peers[0]?.audio).toBe(boosted);
      leaveVoice();
      expect(created[0]?.state).toBe("closed");
    } finally {
      (globalThis as unknown as { AudioContext?: unknown }).AudioContext = Prev;
    }
  });

  it("restores the previous sender when a gain insert becomes stale", async () => {
    class FakeGain {
      gain = { value: 1 };
      connect(): void {}
      disconnect(): void {}
    }
    class FakeCtx {
      state: AudioContextState = "running";
      resume = async (): Promise<void> => {};
      close = async (): Promise<void> => {
        this.state = "closed";
      };
      createMediaStreamSource(): { connect(): void } {
        return { connect() {} };
      }
      createGain(): FakeGain {
        return new FakeGain();
      }
      createMediaStreamDestination(): { stream: MediaStream } {
        return { stream: fakeStream("gain-dest") };
      }
    }
    const contexts: FakeCtx[] = [];
    const Prev = (globalThis as unknown as { AudioContext?: unknown })
      .AudioContext;
    (globalThis as unknown as { AudioContext: unknown }).AudioContext =
      class extends FakeCtx {
        constructor() {
          super();
          contexts.push(this);
        }
      };
    try {
      const replaceGate = deferred();
      const replaceStarted = deferred();
      const { peers, getUserMediaCalls } = install({
        failMedia: (callIndex) => callIndex === 1,
      });
      joinVoice({
        serverId: "srv",
        channelId: "voice",
        channelName: "Lounge",
      });
      await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
      const previous = peers[0]?.audio;
      const sender = peers[0]?.senderRows.find(
        (s) => s.track?.kind === "audio",
      );
      const replace = sender?.replaceTrack?.bind(sender);
      expect(replace).toBeTruthy();
      let replacements = 0;
      sender!.replaceTrack = async (next) => {
        replacements += 1;
        if (replacements === 1) {
          replaceStarted.resolve();
          await replaceGate.promise;
        }
        await replace!(next);
      };

      useMediaSettings.getState().patch({ inputGain: 0.5 });
      await replaceStarted.promise;
      useMediaSettings.getState().patch({ audioInputId: "missing-mic" });
      await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
      replaceGate.resolve();

      await vi.waitFor(() => expect(replacements).toBe(2));
      expect(sender?.track).toBe(previous);
      expect(peers[0]?.audio).toBe(previous);
      expect(trackStopped(previous)).toBe(false);
      expect(contexts[0]?.state).toBe("closed");
    } finally {
      (globalThis as unknown as { AudioContext?: unknown }).AudioContext = Prev;
    }
  });

  it("rolls back a processor that fails during replaceTrack and commits a healthy browser fallback", async () => {
    const contexts: Context[] = [];
    class Context {
      state = "running";
      onstatechange: (() => void) | null = null;
      constructor() {
        contexts.push(this);
      }
      resume = async () => {};
      close = async () => {
        this.state = "closed";
      };
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createGain() {
        return { gain: { value: 1 }, connect() {}, disconnect() {} };
      }
      createMediaStreamDestination() {
        return { stream: fakeStream(`processed-${contexts.length}`) };
      }
    }
    const previousContext = globalThis.AudioContext;
    (globalThis as unknown as { AudioContext: unknown }).AudioContext = Context;
    try {
      const env = install();
      useMediaSettings
        .getState()
        .patch({ processingMode: "browser", inputGain: 0.5 });
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
      await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
      const sender = env.peers[0]!.senderRows[0]!,
        previous = sender.track;
      const replace = sender.replaceTrack!.bind(sender),
        gate = deferred(),
        started = deferred();
      const replacements: Array<MediaStreamTrack | null> = [];
      sender.replaceTrack = async (next) => {
        replacements.push(next);
        if (replacements.length === 1) {
          started.resolve();
          await gate.promise;
        }
        await replace(next);
      };
      useMediaSettings.getState().patch({ audioInputId: "new-mic" });
      await started.promise;
      const candidate = contexts[1]!;
      candidate.state = "closed";
      candidate.onstatechange?.(); // It is not yet active: the persistent usable state must catch this.
      gate.resolve();
      await vi.waitFor(() => expect(env.getUserMediaCalls()).toBe(3));
      await vi.waitFor(() =>
        expect(sender.track).toBe(env.streams[2]!.getAudioTracks()[0]),
      );
      expect(replacements[1]).toBe(previous);
      expect(trackStopped(env.streams[1]!.getAudioTracks()[0])).toBe(true);
      expect(trackStopped(replacements[0])).toBe(true);
      expect(
        env.errors.some(
          (error) =>
            error instanceof Error && error.message.includes("Browser-Ersatz"),
        ),
      ).toBe(true);
      expect(useVoice.getState().status).toBe("joined");
    } finally {
      globalThis.AudioContext = previousContext;
    }
  });

  it("adopts native fallback without rebuilding the rejected gain context", async () => {
    const contexts: Array<{ state: string }> = [];
    class Context {
      state = "suspended";
      constructor() {
        contexts.push(this);
      }
      resume = async () => {
        throw new Error("resume rejected");
      };
      close = async () => {
        this.state = "closed";
      };
    }
    const previousContext = globalThis.AudioContext,
      previousWorklet = globalThis.AudioWorkletNode;
    (globalThis as unknown as { AudioContext: unknown }).AudioContext = Context;
    (globalThis as unknown as { AudioWorkletNode: unknown }).AudioWorkletNode =
      class {};
    try {
      const env = install();
      useMediaSettings.getState().patch({ inputGain: 1.5 });
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
      await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
      expect(env.getUserMediaCalls()).toBe(2);
      expect(contexts).toHaveLength(1);
      expect(env.peers[0]!.senderRows[0]!.track).toBe(
        env.streams[1]!.getAudioTracks()[0],
      );
      expect(trackStopped(env.streams[0]!.getAudioTracks()[0])).toBe(true);
      expect(trackStopped(env.streams[1]!.getAudioTracks()[0])).toBe(false);
      expect(useAudioProcessing.getState()).toMatchObject({
        actual: "browser",
        inputGain: 1,
        contextState: null,
      });
    } finally {
      globalThis.AudioContext = previousContext;
      globalThis.AudioWorkletNode = previousWorklet;
    }
  });

  it("does not let a retired processor failure detach a healthy mic replacement", async () => {
    const contexts: Context[] = [];
    class Context {
      state = "running";
      onstatechange: (() => void) | null = null;
      constructor() {
        contexts.push(this);
      }
      resume = async () => {};
      close = async () => {
        this.state = "closed";
      };
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createGain() {
        return { gain: { value: 1 }, connect() {}, disconnect() {} };
      }
      createMediaStreamDestination() {
        return { stream: fakeStream(`processed-${contexts.length}`) };
      }
    }
    const previousContext = globalThis.AudioContext;
    (globalThis as unknown as { AudioContext: unknown }).AudioContext = Context;
    try {
      const env = install({ failMedia: (index) => index === 2 });
      useMediaSettings
        .getState()
        .patch({ processingMode: "browser", inputGain: 0.5 });
      joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
      await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
      const sender = env.peers[0]!.senderRows[0]!,
        replace = sender.replaceTrack!.bind(sender),
        gate = deferred();
      const replacements: Array<MediaStreamTrack | null> = [];
      sender.replaceTrack = async (next) => {
        replacements.push(next);
        if (replacements.length === 1) {
          await gate.promise;
        }
        await replace(next);
      };
      useMediaSettings.getState().patch({ audioInputId: "healthy-next-mic" });
      await vi.waitFor(() => expect(replacements).toHaveLength(1));
      contexts[0]!.state = "closed";
      contexts[0]!.onstatechange?.();
      gate.resolve();
      await new Promise((resolve) => setTimeout(resolve, 30));
      expect(sender.track?.id).toBe("processed-2-a");
      expect(replacements).toHaveLength(1);
      expect(env.getUserMediaCalls()).toBe(2);
      expect(trackStopped(env.streams[1]!.getAudioTracks()[0])).toBe(false);
    } finally {
      globalThis.AudioContext = previousContext;
    }
  });

  it("keeps the newest mic when overlapping device picks finish out of order", async () => {
    const gates = [deferred(), deferred(), deferred()];
    const { peers, streams, getUserMediaCalls, errors } = install({
      gateMedia: async (i) => {
        await gates[i]?.promise;
      },
      mediaStreamFor: (i, constraints) => {
        if (constraints.video) return fakeVideoStream(`cam-${i}`);
        return fakeStream(`mic-${i}`);
      },
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(1));
    gates[0]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    const initial = peers[0]?.audio;

    useMediaSettings.getState().patch({ audioInputId: "mic-a" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    useMediaSettings.getState().patch({ audioInputId: "mic-b" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(3));

    gates[2]!.resolve(); // B first
    await vi.waitFor(() => expect(peers[0]?.audio).not.toBe(initial));
    const winner = peers[0]?.audio;
    expect(winner?.id).toBe("mic-2-a");

    gates[1]!.resolve(); // late A
    await Promise.resolve();
    await Promise.resolve();
    await vi.waitFor(() => expect(streamStopped(streams[1]!)).toBe(true));
    expect(peers[0]?.audio).toBe(winner);
    expect(trackStopped(winner)).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
    expect(errors).toHaveLength(0);
  });

  it("stops a late mic capture after leave and does not throw on null peer", async () => {
    const gates = [deferred(), deferred()];
    const { peers, streams, getUserMediaCalls, errors } = install({
      gateMedia: async (i) => {
        await gates[i]?.promise;
      },
      mediaStreamFor: (i, constraints) => {
        if (constraints.video) return fakeVideoStream(`cam-${i}`);
        return fakeStream(`mic-${i}`);
      },
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(1));
    gates[0]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());

    useMediaSettings.getState().patch({ audioInputId: "mic-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    leaveVoice();
    expect(useVoice.getState().status).toBe("idle");
    expect(peers[0]?.closed).toBe(true);

    gates[1]!.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await vi.waitFor(() => expect(streamStopped(streams[1]!)).toBe(true));
    expect(useVoice.getState().status).toBe("idle");
    expect(errors).toHaveLength(0);
  });

  it("does not turn the camera back on when a device switch finishes after off", async () => {
    const gates: Array<ReturnType<typeof deferred> | undefined> = [];
    const { peers, streams, getUserMediaCalls, errors } = install({
      gateMedia: async (i) => {
        while (gates.length <= i) gates.push(deferred());
        await gates[i]!.promise;
      },
      mediaStreamFor: (i, constraints) => {
        if (constraints.video) return fakeVideoStream(`cam-${i}`);
        return fakeStream(`mic-${i}`);
      },
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(1));
    gates[0]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());

    toggleCamera();
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    gates[1]!.resolve();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    expect(useVoice.getState().camera).toBe(true);

    useMediaSettings.getState().patch({ videoInputId: "cam-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(3));
    toggleCamera();
    expect(useVoice.getState().camera).toBe(false);
    expect(useVoice.getState().localCamera).toBeNull();

    gates[2]!.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await vi.waitFor(() => expect(streamStopped(streams[2]!)).toBe(true));
    expect(useVoice.getState().camera).toBe(false);
    expect(useVoice.getState().localCamera).toBeNull();
    expect(errors).toHaveLength(0);
  });

  it("does not let the initial mic overwrite a device pick that finished first", async () => {
    const gates = [deferred(), deferred()];
    const { peers, streams, getUserMediaCalls } = install({
      gateMedia: async (i) => {
        await gates[i]?.promise;
      },
      mediaStreamFor: (i, constraints) => {
        if (constraints.video) return fakeVideoStream(`cam-${i}`);
        return fakeStream(`mic-${i}`);
      },
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(1));
    // Peer exists before initial gum resolves, so a device change can race it.
    await vi.waitFor(() => expect(peers.length).toBe(1));
    useMediaSettings.getState().patch({ audioInputId: "mic-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    gates[1]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio?.id).toBe("mic-1-a"));
    const winner = peers[0]?.audio;
    gates[0]!.resolve();
    await Promise.resolve();
    await Promise.resolve();
    await vi.waitFor(() => expect(streamStopped(streams[0]!)).toBe(true));
    expect(peers[0]?.audio).toBe(winner);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("keeps the previous mic when replaceTrack rejects", async () => {
    const { peers, getUserMediaCalls, errors } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    const first = peers[0]?.audio;
    const sender = peers[0]?.senderRows.find((s) => s.track?.kind === "audio");
    expect(sender?.replaceTrack).toBeTruthy();
    sender!.replaceTrack = async () => {
      throw new Error("replace failed");
    };
    useMediaSettings.getState().patch({ audioInputId: "mic-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    await vi.waitFor(() => expect(errors.length).toBeGreaterThan(0));
    expect(peers[0]?.audio).toBe(first);
    expect(trackStopped(first)).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("applies mute to a mic that finishes after mute was toggled", async () => {
    const gates = [deferred(), deferred()];
    const { peers, getUserMediaCalls } = install({
      gateMedia: async (i) => {
        await gates[i]?.promise;
      },
      mediaStreamFor: (i, constraints) => {
        if (constraints.video) return fakeVideoStream(`cam-${i}`);
        return fakeStream(`mic-${i}`);
      },
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(1));
    gates[0]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    useMediaSettings.getState().patch({ audioInputId: "mic-usb" });
    await vi.waitFor(() => expect(getUserMediaCalls()).toBe(2));
    toggleMute();
    expect(useVoice.getState().muted).toBe(true);
    gates[1]!.resolve();
    await vi.waitFor(() => expect(peers[0]?.audio?.id).toBe("mic-1-a"));
    expect(peers[0]?.audio?.enabled).toBe(false);
  });

  it("stops diagnostics on leave, rejoin, watch, and logout", async () => {
    const env = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(diagnosticsPolling().voice).toBe(true));
    leaveVoice();
    expect(diagnosticsPolling().voice).toBe(false);
    expect(
      useVoiceDiagnostics.getState().phases.map((phase) => phase.phase),
    ).toContain("voice-only");

    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(diagnosticsPolling().voice).toBe(true));
    expect(diagnosticsPolling().watch).toBe(false);
    leaveVoice();
    expect(diagnosticsPolling().voice).toBe(false);

    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(diagnosticsPolling().watch).toBe(true));
    stopWatching();
    expect(diagnosticsPolling().watch).toBe(false);

    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(diagnosticsPolling().voice).toBe(true));
    useSession.setState({
      status: "authenticated",
      user: {
        id: "u-self",
        email: "ada@example.com",
        name: "Ada",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00.000Z",
      },
    });
    useSession.setState({ status: "anonymous", user: null });
    expect(diagnosticsPolling().voice).toBe(false);
    expect(JSON.stringify(buildDiagnosticExport())).not.toContain(
      "ada@example.com",
    );
    expect(JSON.stringify(buildDiagnosticExport())).not.toContain(
      "abcdefghjkmn",
    );
    resetSessionForTests();
    expect(env.peers.length).toBeGreaterThan(0);
  });

  it("records stream and device changes without the device id", async () => {
    const { peers } = install();
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(peers[0]?.audio).toBeTruthy());
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    toggleCamera();
    expect(useVoice.getState().camera).toBe(false);
    useMediaSettings.getState().patch({ audioInputId: "mic-secret" });
    const events = useVoiceDiagnostics.getState().events;
    expect(
      events.some(
        (event) => event.kind === "stream-start" && event.detail === "camera",
      ),
    ).toBe(true);
    expect(
      events.some(
        (event) => event.kind === "stream-stop" && event.detail === "camera",
      ),
    ).toBe(true);
    expect(
      events.some(
        (event) =>
          event.kind === "device-change" && event.detail === "audio-input",
      ),
    ).toBe(true);
    const phases = useVoiceDiagnostics
      .getState()
      .phases.map((phase) => phase.phase);
    expect(phases).toEqual(["voice-only", "stream-on", "stream-off"]);
    expect(JSON.stringify(events)).not.toContain("mic-secret");
  });
});

describe("bounded Live lease recovery", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    vi.useRealTimers();
  });
  const flush = async () => {
    for (let i = 0; i < 30; i++) await Promise.resolve();
  };
  async function joined(options: Parameters<typeof install>[0]) {
    const env = install(options);
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    vi.useFakeTimers();
    return env;
  }

  it("retries only live_busy with one retained capture, epoch and nonce, then publishes parent before audio", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    let attempts = 0;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" && ++attempts <= 2 ? new Error("live_busy") : undefined,
    });
    const mic = env.peers[0]!.audio;
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    expect(stream).toBeTruthy();
    expect(useVoice.getState().live).toBe(true);
    expect(streamStopped(stream)).toBe(false);
    expect(env.peers[0]!.sender("l")).toBeUndefined();
    expect(env.peers[0]!.sender("la")).toBeUndefined();
    expect(attempts).toBe(1);
    await vi.advanceTimersByTimeAsync(400);
    expect(attempts).toBe(3);
    const inputs = env.peers[0]!.publicationInputs.filter(
      (input) => input.kind === "l",
    );
    expect(inputs).toHaveLength(3);
    expect(
      inputs.every(
        (input) =>
          input.track === stream.getVideoTracks()[0] &&
          input.epoch === inputs[0]!.epoch &&
          input.lc === inputs[0]!.lc,
      ),
    ).toBe(true);
    expect(env.peers[0]!.sender("l")?.track).toBe(stream.getVideoTracks()[0]);
    expect(env.peers[0]!.sender("la")?.track).toBe(stream.getAudioTracks()[0]);
    expect(
      env.peers[0]!.publicationInputs.find((input) => input.kind === "la")
        ?.parent,
    ).toBe(env.peers[0]!.sender("l")?.producerId);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(env.peers[0]!.audio).toBe(mic);
    expect(env.errors).toHaveLength(0);
  });

  it("never retries genuine forbidden and keeps the microphone while cleaning the denied Live source", async () => {
    let attempts = 0;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" ? (attempts++, new Error("forbidden")) : undefined,
    });
    const mic = env.peers[0]!.audio;
    toggleGoLive();
    await flush();
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts).toBe(1);
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().localLive).toBeNull();
    expect(env.peers[0]!.sender("l")).toBeUndefined();
    expect(env.peers[0]!.audio).toBe(mic);
    expect(trackStopped(mic)).toBe(false);
    expect(env.errors).toHaveLength(1);
  });

  it("Stop during live_busy cancels every later publication without recapturing", async () => {
    let attempts = 0;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" ? (attempts++, new Error("live_busy")) : undefined,
    });
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    expect(streamStopped(stream)).toBe(false);
    toggleGoLive();
    await vi.advanceTimersByTimeAsync(1000);
    expect(attempts).toBe(1);
    expect(streamStopped(stream)).toBe(true);
    expect(env.peers[0]!.sender("l")).toBeUndefined();
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(useVoice.getState().live).toBe(false);
    expect(env.errors).toHaveLength(0);
  });

  it("recovery replaces the peer generation while preserving the pending Live capture and UUID", async () => {
    let busy = true;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" && busy ? new Error("live_busy") : undefined,
    });
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    const initial = env.peers[0]!.publicationInputs.find(
      (input) => input.kind === "l",
    )!;
    expect(streamStopped(stream)).toBe(false);
    busy = false;
    env.closeMedia();
    await vi.advanceTimersByTimeAsync(1000);
    expect(env.peers).toHaveLength(2);
    expect(
      env.peers[0]!.publicationInputs.filter((input) => input.kind === "l"),
    ).toHaveLength(1);
    const replacement = env.peers[1]!.publicationInputs.find(
      (input) => input.kind === "l",
    )!;
    expect(replacement.epoch).toBe(initial.epoch);
    expect(replacement.lc).toBe(initial.lc);
    expect(replacement.track).toBe(initial.track);
    expect(env.peers[1]!.sender("l")?.track).toBe(stream.getVideoTracks()[0]);
    expect(useVoice.getState().localLive).toBe(stream);
    expect(streamStopped(stream)).toBe(false);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(env.getUserMediaCalls()).toBe(1);
  });

  it("a replacement claim prevents an old busy retry from publishing or ending the current source", async () => {
    let busy = true;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" && busy ? new Error("live_busy") : undefined,
    });
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    const initial = env.peers[0]!.publicationInputs.find(
      (input) => input.kind === "l",
    )!;
    busy = false;
    const next = "00000000-0000-0000-0000-000000000002";
    env.emitSig({
      op: "sig",
      t: "p",
      s: "srv",
      c: "voice",
      u: "u-self",
      k: "l",
      lc: next,
    });
    await flush();
    const sender = env.peers[0]!.sender("l");
    await vi.advanceTimersByTimeAsync(1000);
    expect(env.peers[0]!.sender("l")).toBe(sender);
    expect(sender?.track).toBe(stream.getVideoTracks()[0]);
    expect(
      env.peers[0]!.publicationInputs.filter(
        (input) => input.kind === "l" && input.lc === initial.lc,
      ),
    ).toHaveLength(1);
    expect(useVoice.getState().localLive).toBe(stream);
    expect(streamStopped(stream)).toBe(false);
    expect(env.getDisplayMediaCalls()).toBe(1);
  });

  it("exhausts one total ten-second busy budget, releases Live resources once and retains microphone", async () => {
    let attempts = 0;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" ? (attempts++, new Error("live_busy")) : undefined,
    });
    const mic = env.peers[0]!.audio;
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    expect(streamStopped(stream)).toBe(false);
    await vi.advanceTimersByTimeAsync(9999);
    expect(useVoice.getState().live).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    expect(attempts).toBe(50);
    expect(useVoice.getState().live).toBe(false);
    expect(streamStopped(stream)).toBe(true);
    expect(env.peers[0]!.senders()).toHaveLength(1);
    expect(env.peers[0]!.audio).toBe(mic);
    expect(trackStopped(mic)).toBe(false);
    expect(env.peers).toHaveLength(1);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(env.errors).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(10000);
    expect(attempts).toBe(50);
  });

  it("a hanging SDK attempt after busy shares the original deadline and cannot issue a late producer RPC", async () => {
    let busy = true;
    const env = await joined({
      produceError: (kind) =>
        kind === "l" && busy ? new Error("live_busy") : undefined,
    });
    const gate = deferred();
    const peer = env.peers[0]!,
      original = peer.publish.bind(peer);
    let starts = 0;
    peer.publish = async (input) => {
      if (input.kind === "l" && ++starts > 1) await gate.promise;
      return original(input);
    };
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    busy = false;
    await vi.advanceTimersByTimeAsync(200);
    expect(starts).toBe(2);
    await vi.advanceTimersByTimeAsync(9799);
    expect(useVoice.getState().live).toBe(true);
    await vi.advanceTimersByTimeAsync(1);
    expect(useVoice.getState().live).toBe(false);
    expect(streamStopped(stream)).toBe(true);
    gate.resolve();
    await flush();
    expect(
      env.mediaSent.filter(
        (frame) => frame.op === "produce" && frame.k === "l",
      ),
    ).toHaveLength(1);
    expect(peer.sender("l")).toBeUndefined();
    expect(trackStopped(peer.audio)).toBe(false);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(env.errors).toHaveLength(1);
  });

  it("an acknowledged but delayed SDK result after Stop never revives the source", async () => {
    const env = await joined({});
    const gate = deferred();
    const peer = env.peers[0]!,
      original = peer.publish.bind(peer);
    peer.publish = async (input) => {
      const sender = await original(input);
      if (input.kind === "l") await gate.promise;
      return sender;
    };
    toggleGoLive();
    await flush();
    const stream = useVoice.getState().localLive!;
    const actualProducer = peer.sender("l")?.producerId;
    expect(actualProducer).toBeTruthy();
    toggleGoLive();
    gate.resolve();
    await flush();
    expect(peer.sender("l")).toBeUndefined();
    expect(streamStopped(stream)).toBe(true);
    expect(useVoice.getState().live).toBe(false);
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "la"),
    ).toBe(false);
    expect(
      env.mediaSent.some(
        (frame) =>
          frame.op === "closeProducer" && frame.producerId === actualProducer,
      ),
    ).toBe(true);
    expect(env.getDisplayMediaCalls()).toBe(1);
  });

  it("producer-ID scoped late cleanup cannot close a new Live capture started after Stop", async () => {
    const env = await joined({});
    const gate = deferred();
    const peer = env.peers[0]!,
      original = peer.publish.bind(peer);
    let held = false;
    peer.publish = async (input) => {
      const sender = await original(input);
      if (input.kind === "l" && !held) {
        held = true;
        await gate.promise;
      }
      return sender;
    };
    toggleGoLive();
    await flush();
    const oldStream = useVoice.getState().localLive!;
    toggleGoLive();
    toggleGoLive();
    await flush();
    const newStream = useVoice.getState().localLive!,
      newSender = peer.sender("l");
    expect(newStream).not.toBe(oldStream);
    expect(newSender?.track).toBe(newStream.getVideoTracks()[0]);
    gate.resolve();
    await flush();
    expect(peer.sender("l")).toBe(newSender);
    expect(streamStopped(oldStream)).toBe(true);
    expect(streamStopped(newStream)).toBe(false);
    expect(useVoice.getState().localLive).toBe(newStream);
    expect(useVoice.getState().live).toBe(true);
    expect(env.getDisplayMediaCalls()).toBe(2);
    expect(env.errors).toHaveLength(0);
  });
});

describe("source and transport continuity", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    vi.useRealTimers();
  });

  async function connected(options?: Parameters<typeof install>[0]) {
    const env = install(options);
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    return env;
  }

  it("keeps all capture tracks when only the media socket reconnects", async () => {
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );

    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );

    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());

    const before = useVoice.getState();
    const mic = env.peers[0]!.audio;
    env.closeMedia();
    await vi.waitFor(() => expect(env.peers).toHaveLength(2));
    for (const key of ["localCamera", "localScreen", "localLive"] as const) {
      expect(useVoice.getState()[key]).toBe(before[key]);
      expect(streamStopped(before[key])).toBe(false);
      expect(
        env.peers[1]!.senderRows.some(
          (sender) => sender.track === before[key]!.getVideoTracks()[0],
        ),
      ).toBe(true);
    }
    expect(env.getDisplayMediaCalls()).toBe(2);
    expect(env.getUserMediaCalls()).toBe(2);
    expect(env.peers[1]!.audio).toBe(mic);
    leaveVoice();
    for (const key of ["localCamera", "localScreen", "localLive"] as const)
      expect(streamStopped(before[key])).toBe(true);
    expect(trackStopped(mic)).toBe(true);
  });

  it("releases the Live claim when media recovery abandons an open picker", async () => {
    const picker = deferred();
    const late = fakeVideoStream("late-live");
    const env = await connected({
      holdDisplay: picker.promise,
      displayStreamFor: () => late,
    });
    toggleGoLive();
    await vi.waitFor(() =>
      expect(useVoice.getState().participants["u-self"]?.pubs).toContain("l"),
    );
    env.closeMedia();
    await vi.waitFor(() => expect(env.peers[1]?.audio).toBeTruthy());
    expect(useVoice.getState().live).toBe(false);
    expect(useVoiceRoster.getState().live.srv?.voice).toBeUndefined();
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain("l");

    // The picker answers after the rebuild: nothing may come back.
    picker.resolve();
    await vi.waitFor(() => expect(streamStopped(late)).toBe(true));
    expect(useVoice.getState().live).toBe(false);
    expect(useVoice.getState().localLive).toBeNull();
    expect(liveClaimFrames(env.sent)).toEqual(["p", "u"]);
    expect(env.peers[1]!.sender("l")).toBeUndefined();
    expect(env.errors).toHaveLength(0);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("bounds failed ticket retries and reports one failure", async () => {
    const env = install({
      ticketError: (i) => (i > 0 ? new Error("offline") : undefined),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.closeMedia();
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.ticketCalls()).toBe(8);
    expect(env.peers).toHaveLength(1);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(env.errors).toHaveLength(1);
    expect(useVoice.getState().status).toBe("idle");
    expect(vi.getTimerCount()).toBe(0);
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.ticketCalls()).toBe(8);
  });

  it("cancels pending transport recovery on leave", async () => {
    const gate = deferred();
    const env = install({
      gateTicket: (i) => (i > 0 ? gate.promise : undefined),
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.closeMedia();
    await vi.advanceTimersByTimeAsync(300);
    expect(env.ticketCalls()).toBe(2);
    leaveVoice();
    gate.resolve();
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.peers).toHaveLength(1);
    expect(env.streams.every(streamStopped)).toBe(true);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("leaves ended display capture off until the next explicit toggle", async () => {
    const env = await connected();
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    const screen = useVoice.getState().localScreen!;
    Object.defineProperty(screen.getVideoTracks()[0], "readyState", {
      value: "ended",
    });
    env.closeMedia();
    await vi.waitFor(() => expect(env.peers).toHaveLength(2));
    expect(useVoice.getState().sharing).toBe(false);
    expect(useVoice.getState().localScreen).toBeNull();
    expect(env.getDisplayMediaCalls()).toBe(1);
    toggleShare();
    await vi.waitFor(() => expect(env.getDisplayMediaCalls()).toBe(2));
  });

  it("uses freshly minted TURN credentials on transport replacement", async () => {
    const env = install({
      iceServersFor: (i) => [
        {
          urls: ["turn:127.0.0.1:3478"],
          username: `lease-${i}`,
          credential: `test-${i}`,
        },
      ],
    });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    expect(env.peers[0]!.iceServers[0]?.username).toBe("lease-0");
    env.closeMedia();
    await vi.waitFor(() => expect(env.peers).toHaveLength(2));
    expect(env.peers[1]!.iceServers[0]?.username).toBe("lease-1");
    expect(env.getUserMediaCalls()).toBe(1);
  });

  it("bounds watch recovery and cancels its timers without capture prompts", async () => {
    const env = install({
      ticketError: (i) => (i > 0 ? new Error("offline") : undefined),
    });
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.closeMedia();
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.ticketCalls()).toBe(8);
    expect(useVoice.getState().watching).toBe(false);
    expect(env.errors).toHaveLength(1);
    expect(env.getUserMediaCalls()).toBe(0);
    expect(env.getDisplayMediaCalls()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("retries an unavailable Watch recovery Join with the remaining budget", async () => {
    const env = install();
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.closeMedia();
    await vi.advanceTimersByTimeAsync(350);
    expect(env.peers).toHaveLength(2);
    env.emitMedia({ op: "err", e: "unavailable" });
    expect(useVoice.getState().watching).toBe(true);
    expect(env.peers[1]!.closed).toBe(true);
    await vi.advanceTimersByTimeAsync(650);
    expect(env.peers).toHaveLength(3);

    await vi.advanceTimersByTimeAsync(0);
    env.peers[2]!.setTransportState("send", "connected");
    expect(useVoice.getState().watching).toBe(true);
    expect(env.errors).toHaveLength(0);
    expect(env.getUserMediaCalls()).toBe(0);
    stopWatching();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("bounds repeated unavailable Watch recovery Joins and reports once", async () => {
    const env = install();
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.closeMedia();
    for (let attempt = 1; attempt <= 7; attempt++) {
      await vi.advanceTimersByTimeAsync(5000);
      expect(env.peers).toHaveLength(attempt + 1);
      env.emitMedia({ op: "err", e: "unavailable" });
    }
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.ticketCalls()).toBe(8);
    expect(useVoice.getState().watching).toBe(false);
    expect(env.errors).toHaveLength(1);
    expect(env.getUserMediaCalls()).toBe(0);
    expect(env.getDisplayMediaCalls()).toBe(0);
    expect(vi.getTimerCount()).toBe(0);
  });

  it.each(["unauthorized", "forbidden", "bad_request", "negotiation_failed"])(
    "ends Watch recovery on terminal %s without retrying",
    async (code) => {
      const env = install();
      watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
      await vi.waitFor(() => expect(env.peers).toHaveLength(1));
      vi.useFakeTimers();
      env.closeMedia();
      await vi.advanceTimersByTimeAsync(350);
      env.emitMedia({ op: "err", e: code });
      await vi.advanceTimersByTimeAsync(60000);
      expect(env.ticketCalls()).toBe(2);
      expect(useVoice.getState().watching).toBe(false);
      expect(env.errors).toHaveLength(1);
      expect(vi.getTimerCount()).toBe(0);
    },
  );

  it("ends an initial unavailable Watch Join without starting recovery", async () => {
    const env = install();
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(1));
    vi.useFakeTimers();
    env.emitMedia({ op: "err", e: "unavailable" });
    await vi.advanceTimersByTimeAsync(60000);
    expect(env.ticketCalls()).toBe(1);
    expect(useVoice.getState().watching).toBe(false);
    expect(env.errors).toHaveLength(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("shares the video budget between camera and screen without lowering audio bitrate", async () => {
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(
        env.peers[0]?.senderRows.filter((s) => s.track?.kind === "video"),
      ).toHaveLength(1),
    );
    toggleShare();
    await vi.waitFor(() =>
      expect(
        env.peers[0]?.senderRows.filter((s) => s.track?.kind === "video"),
      ).toHaveLength(2),
    );
    const video = env.peers[0]!.senderRows.filter(
      (s) => s.track?.kind === "video",
    );
    await vi.waitFor(() =>
      expect(
        video.map((s) => s.getParameters!().encodings[0]?.maxBitrate),
      ).toEqual([undefined, undefined]),
    );
    expect(
      video.map((s) => s.getParameters!().encodings[0]?.maxFramerate),
    ).toEqual([30, 30]);
    const audio = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "audio",
    )!;
    expect(audio.getParameters!().encodings[0]?.maxBitrate).toBeUndefined();
    toggleShare();
    await vi.waitFor(() =>
      expect(
        video[0]!.getParameters!().encodings[0]?.maxBitrate,
      ).toBeUndefined(),
    );
  });

  it("does not toast each ICE failure but still leaves on unauthorized", async () => {
    const env = await connected();
    for (let i = 0; i < 12; i++) env.emitMedia({ op: "err", e: "ice_failed" });
    expect(useVoice.getState().status).toBe("joined");
    expect(env.peers[0]?.closed).toBe(false);
    expect(env.errors).toHaveLength(0);
    env.emitMedia({ op: "err", e: "unauthorized" });
    expect(useVoice.getState().status).toBe("idle");
  });

  it("does not close the watch peer on a duplicate transport failure", async () => {
    const env = await connected();
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers[1]).toBeTruthy());
    env.peers[1]!.setTransportState("send", "failed");
    env.peers[1]!.setTransportState("send", "failed");
    await Promise.resolve();
    expect(
      useVoiceDiagnostics
        .getState()
        .events.some(
          (event) =>
            event.kind === "ice-error" &&
            event.connection === "watch" &&
            event.detail === "send:failed",
        ),
    ).toBe(true);
    expect(env.peers[1]?.closed).toBe(false);
    expect(useVoice.getState().watching).toBe(true);
    expect(useVoice.getState().status).toBe("joined");
  });

  it("captures camera and screen with the selected profile constraints", async () => {
    useMediaSettings.getState().patch({
      cameraProfile: "economy",
      screenProfile: "detail",
    });
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    expect(env.lastUserMedia()).toEqual({
      audio: false,
      video: videoConstraintsFor("camera", "economy"),
    });
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    expect(env.lastDisplayMedia()).toEqual({
      audio: false,
      video: videoConstraintsFor("screen", "detail"),
    });
    const video = env.peers[0]!.senderRows.filter(
      (s) => s.track?.kind === "video",
    );
    await vi.waitFor(() =>
      expect(
        video.map((s) => s.getParameters!().encodings[0]?.maxBitrate),
      ).toEqual([undefined, undefined]),
    );
    expect(
      video.map((s) => s.getParameters!().encodings[0]?.maxFramerate),
    ).toEqual([15, 30]);

    env.peers[0]!.getStats = async () =>
      video.map((sender, index) => ({
        id: `video-${index}`,
        type: "outbound-rtp",
        kind: "video",
        timestamp: Date.now(),
        trackIdentifier: sender.track!.id,
        bytesSent: 1000,
      }));
    await vi.waitFor(
      () =>
        expect(
          useVoiceDiagnostics.getState().latest?.voice?.flows,
        ).toHaveLength(2),
      { timeout: 3_500 },
    );
    const exported = buildDiagnosticExport();
    const flows = exported.samples.at(-1)!.voice!.flows;
    expect(flows.map((flow) => flow.source)).toEqual(["camera", "screen"]);
    expect(flows.map((flow) => flow.configuredMaxBitrateBps)).toEqual(
      video.map(
        (sender) => sender.getParameters!().encodings[0]!.maxBitrate ?? null,
      ),
    );
    expect(flows.map((flow) => flow.configuredMaxFps)).toEqual([15, 30]);
    expect(exported.samples.at(-1)?.caps.videoSendBudget).toBeNull();
    expect(exported.settings.cameraProfile).toBe("economy");
    expect(exported.settings.screenProfile).toBe("detail");
  });

  it("applies 4K/60 to camera, screen and Go Live and updates the shared upload cap live", async () => {
    useMediaSettings
      .getState()
      .patch({ cameraProfile: "2160p60", screenProfile: "2160p60" });
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    expect(env.lastUserMedia()?.video).toEqual({
      width: { ideal: 3840, max: 3840 },
      height: { ideal: 2160, max: 2160 },
      frameRate: { ideal: 60, max: 60 },
    });
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    expect(env.lastDisplayMedia()?.video).toEqual(
      videoConstraintsFor("screen", "2160p60"),
    );
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    expect(env.lastDisplayMedia()?.video).toEqual(
      videoConstraintsFor("screen", "2160p60"),
    );
    const video = () =>
      env.peers[0]!.senderRows.filter(
        (sender) => sender.track?.kind === "video",
      );
    await vi.waitFor(() =>
      expect(
        video().map((sender) => sender.getParameters!().encodings[0]),
      ).toEqual(Array(3).fill({ maxFramerate: 60, priority: "low" })),
    );
    useMediaSettings.getState().patch({ videoUploadLimit: 9_000_000 });
    await vi.waitFor(() =>
      expect(
        video().map((sender) =>
          sender.getParameters!().encodings.reduce(
            (sum, e) => sum + (e.maxBitrate ?? 0),
            0,
          ),
        ),
      ).toEqual([3_000_000, 3_000_000, 3_000_000]),
    );
    const screen = useVoice.getState().localScreen!.getVideoTracks()[0]!;
    const applied = vi.fn(async () => {});
    screen.applyConstraints = applied;
    useMediaSettings.getState().patch({ screenProfile: "1440p24" });
    await vi.waitFor(() =>
      expect(applied).toHaveBeenCalledWith(
        videoConstraintsFor("screen", "1440p24"),
      ),
    );
    await vi.waitFor(() =>
      expect(
        video().map(
          (sender) => sender.getParameters!().encodings[0]?.maxFramerate,
        ),
      ).toEqual([60, 24, 24]),
    );
    const total = video().reduce(
      (sum, sender) =>
        sum + (sender.getParameters!().encodings[0]?.maxBitrate ?? 0),
      0,
    );
    expect(total).toBeLessThanOrEqual(9_000_000);
    expect(
      env.peers[0]!.senderRows.find((sender) => sender.track?.kind === "audio")
        ?.getParameters!().encodings[0]?.maxBitrate,
    ).toBeUndefined();
    expect(useVoice.getState().status).toBe("joined");
  });

  it("uses the screen profile for Go Live", async () => {
    useMediaSettings.getState().patch({ screenProfile: "economy" });
    const env = await connected();
    toggleGoLive();
    await vi.waitFor(() => expect(useVoice.getState().localLive).toBeTruthy());
    expect(env.lastDisplayMedia()).toEqual({
      audio: false,
      video: videoConstraintsFor("screen", "economy"),
    });
    const video = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "video",
    );
    await vi.waitFor(() =>
      expect(video?.getParameters?.().encodings[0]?.maxBitrate).toBeUndefined(),
    );
    expect(video?.getParameters?.().encodings[0]?.maxFramerate).toBe(15);
  });

  it("applies a live profile on the sender and the track when the browser allows it", async () => {
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    const track = useVoice.getState().localCamera!.getVideoTracks()[0]!;
    let applied: MediaTrackConstraints | undefined;
    (
      track as MediaStreamTrack & {
        applyConstraints?: (next: MediaTrackConstraints) => Promise<void>;
      }
    ).applyConstraints = async (next) => {
      applied = next;
    };
    useMediaSettings.getState().patch({ cameraProfile: "detail" });
    await vi.waitFor(() =>
      expect(useMediaSettings.getState().cameraProfileApply).toBe("live"),
    );
    expect(applied).toEqual(videoConstraintsFor("camera", "detail"));
    const video = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "video",
    );
    expect(video?.getParameters?.().encodings[0]?.maxBitrate).toBeUndefined();
    expect(video?.getParameters?.().encodings[0]?.maxFramerate).toBe(30);
    const audio = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "audio",
    );
    expect(audio?.getParameters?.().encodings[0]?.maxBitrate).toBeUndefined();
    expect(useVoice.getState().muted).toBe(false);
    expect(useVoice.getState().deafened).toBe(false);
  });

  it("marks the next stream when applyConstraints is rejected and still caps the sender", async () => {
    const env = await connected();
    toggleMute();
    toggleCamera();
    await vi.waitFor(() =>
      expect(useVoice.getState().localCamera).toBeTruthy(),
    );
    const track = useVoice.getState().localCamera!.getVideoTracks()[0]!;
    (
      track as MediaStreamTrack & {
        applyConstraints?: (next: MediaTrackConstraints) => Promise<void>;
      }
    ).applyConstraints = async () => {
      const error = new Error("rejected");
      error.name = "OverconstrainedError";
      throw error;
    };
    useMediaSettings.getState().patch({
      cameraProfile: "economy",
      quality: "high",
    });
    await vi.waitFor(() =>
      expect(useMediaSettings.getState().cameraProfileApply).toBe("next"),
    );
    const video = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "video",
    );
    expect(video?.getParameters?.().encodings[0]?.maxBitrate).toBeUndefined();
    expect(video?.getParameters?.().encodings[0]?.maxFramerate).toBe(15);
    const audio = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "audio",
    );
    expect(audio?.getParameters?.().encodings[0]?.maxBitrate).toBeUndefined();
    expect(useVoice.getState().muted).toBe(true);
    expect(useVoice.getState().deafened).toBe(false);
    expect(useVoice.getState().camera).toBe(true);
  });

  it("splits one sender share across simulcast encodings", async () => {
    const env = await connected();
    toggleCamera();
    await vi.waitFor(() =>
      expect(
        env.peers[0]?.senderRows.filter((s) => s.track?.kind === "video"),
      ).toHaveLength(1),
    );
    const video = env.peers[0]!.senderRows.find(
      (s) => s.track?.kind === "video",
    )!;
    video.getParameters!().encodings.push({});
    useMediaSettings.getState().patch({ cameraProfile: "economy" });
    await vi.waitFor(() =>
      expect(video.getParameters!().encodings.map((e) => e.maxBitrate)).toEqual(
        [undefined, undefined],
      ),
    );
    expect(video.getParameters!().encodings.map((e) => e.maxFramerate)).toEqual(
      [15, 15],
    );
  });

  it("does not reopen the display picker after rejected capture constraints", async () => {
    useMediaSettings
      .getState()
      .patch({ screenProfile: "detail", sourceAudioShare: "on" });
    const error = new Error("profile rejected");
    error.name = "OverconstrainedError";
    const env = install({ displayError: () => error });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers.length).toBe(1));

    toggleShare();
    await vi.waitFor(() => expect(useVoice.getState().sharing).toBe(false));
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(useVoice.getState().status).toBe("joined");
    expect(useVoice.getState().localScreen).toBeNull();
    expect(env.peers[0]?.closed).toBe(false);
  });

  it("keeps the newer budgets when an older profile update resumes", async () => {
    useMediaSettings.getState().patch({ videoUploadLimit: 2_500_000 });
    const env = await connected();
    toggleCamera();
    toggleShare();
    await vi.waitFor(() =>
      expect(videoBitrates(env.peers[0])).toEqual([1_250_000, 1_250_000]),
    );
    const camera = env.peers[0]!.senderRows.find(
      (sender) => sender.track?.kind === "video",
    )!;
    const held = holdNextSetParameters(camera);
    useMediaSettings.getState().patch({ cameraProfile: "detail" });
    await held.entered;
    useMediaSettings.getState().patch({ cameraProfile: "economy" });
    for (let i = 0; i < 20; i++) await Promise.resolve();
    held.release();
    // Detail then Sparsam, screen stays Ausgewogen. The stale screen share
    // from the Detail pass is 1_538_461.
    await vi.waitFor(() =>
      expect(videoBitrates(env.peers[0])).toEqual(
        allocateVideoBitrates(["economy", "balanced"], 2_500_000),
      ),
    );
    expect(
      env.peers[0]!.senderRows.filter(
        (sender) => sender.track?.kind === "video",
      ).map((sender) => sender.getParameters?.().encodings[0]?.maxFramerate),
    ).toEqual([15, 30]);
  });

  it("applies a stopped sender's freed budget on the same queue", async () => {
    useMediaSettings.getState().patch({ videoUploadLimit: 2_500_000 });
    const env = await connected();
    toggleCamera();
    toggleShare();
    await vi.waitFor(() =>
      expect(videoBitrates(env.peers[0])).toEqual([1_250_000, 1_250_000]),
    );
    const camera = env.peers[0]!.senderRows.find(
      (sender) => sender.track?.kind === "video",
    )!;
    const held = holdNextSetParameters(camera);
    useMediaSettings.getState().patch({ cameraProfile: "detail" });
    await held.entered;
    toggleShare();
    held.release();
    await vi.waitFor(() =>
      expect(videoBitrates(env.peers[0])).toEqual([2_500_000]),
    );
    expect(useVoice.getState().sharing).toBe(false);
    expect(useVoice.getState().camera).toBe(true);
  });
});

describe("authoritative mediasoup session lifecycle", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });
  async function joined(options?: Parameters<typeof install>[0]) {
    const env = install(options);
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Voice" });
    await vi.waitFor(() => expect(env.peers[0]?.started).toBe(true));
    if (options?.media !== false)
      await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    return env;
  }
  function playback() {
    const elements: Playback[] = [];
    class Playback {
      autoplay = false;
      muted = false;
      volume = 1;
      srcObject: MediaStream | null = null;
      attributes: Record<string, string> = {};
      pause = vi.fn();
      play = vi.fn(async () => undefined);
      constructor() {
        elements.push(this);
      }
      setAttribute(k: string, value: string) {
        this.attributes[k] = value;
      }
    }
    vi.stubGlobal("Audio", Playback);
    return elements;
  }
  it("joins and publishes without crypto.randomUUID, as on a plain-http origin", async () => {
    const real = globalThis.crypto;
    vi.stubGlobal("crypto", {
      getRandomValues: (bytes: Uint8Array<ArrayBuffer>) =>
        real.getRandomValues(bytes),
    });
    const env = await joined();
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]!.sender("s")).toBeTruthy());
    const epochs = env.peers[0]!.publicationInputs.map((input) => input.epoch);
    expect(epochs).toHaveLength(2);
    for (const epoch of epochs)
      expect(epoch).toMatch(
        /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
      );
    expect(new Set(epochs).size).toBe(2);
    expect(env.errors).toHaveLength(0);
  });

  it("fails closed before ticket creation when Watch has no current Live publisher", () => {
    const env = install();
    useVoiceRoster.setState({ live: {} });
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    expect(env.ticketCalls()).toBe(0);
    expect(env.mediaSent).toHaveLength(0);
    expect(env.peers).toHaveLength(0);
    expect(useVoice.getState().watching).toBe(false);
    expect(env.errors).toHaveLength(1);
  });
  it("does not downgrade Watch to a seat when the publisher vanishes during ticket acquisition", async () => {
    const gate = deferred();
    const env = install({ gateTicket: () => gate.promise });
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    useVoice.setState({ watchPublisherId: null });
    gate.resolve();
    await vi.waitFor(() => expect(useVoice.getState().watching).toBe(false));
    expect(env.ticketCalls()).toBe(1);
    expect(env.mediaSent).toHaveLength(0);
    expect(env.peers).toHaveLength(0);
    expect(env.getUserMediaCalls()).toBe(0);
  });
  it("keeps Watch receive-only and separate from the seat and rejects a different Live source", async () => {
    const env = await joined();
    const mic = env.peers[0]!.audio;
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers).toHaveLength(2));
    const watch = env.peers[1]!;
    expect(watch.options.role).toBe("watch");
    expect(watch.senderRows).toHaveLength(0);
    expect(env.mediaSent.filter((f) => f.op === "j").at(-1)).toMatchObject({
      w: "u-bob",
      v: 4,
    });
    watch.receive("u-cara", "l", fakeVideoStream("misleading-stream"));
    expect(useVoice.getState().watchStream).toBeNull();
    const selected = fakeVideoStream("wrong-owner:camera");
    watch.receive("u-bob", "l", selected);
    expect(useVoice.getState().watchStream).toBe(selected);
    stopWatching();
    expect(env.mediaSent.at(-1)).toMatchObject({ op: "l" });
    expect(watch.closed).toBe(true);
    expect(useVoice.getState().status).toBe("joined");
    expect(env.peers[0]!.audio).toBe(mic);
    expect(trackStopped(mic)).toBe(false);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(env.getDisplayMediaCalls()).toBe(0);
  });
  it("binds tiles to Consumer owner/k and preserves replacement tiles across stale closures", async () => {
    const env = await joined();
    const seat = env.peers[0]!;
    const first = seat.receive("u-bob", "v", fakeVideoStream("u-cara:s"));
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(first.stream);
    expect(useVoice.getState().remote["u-cara"]).toBeUndefined();
    const current = seat.receive(
      "u-bob",
      "v",
      fakeVideoStream("random-native-stream"),
      "next-camera",
    );
    seat.removeReceived(first);
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(current.stream);
    env.emitSig({ op: "sig", t: "l", s: "srv", c: "voice", u: "u-bob" });
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(current.stream);
    env.emitReady();
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(current.stream);
    seat.removeReceived(current);
    expect(useVoice.getState().remote["u-bob"]?.v).toBeUndefined();
  });
  it("hides an explicitly unpublished source while preserving another publisher", async () => {
    const env = await joined();
    const a = env.peers[0]!.receive("u-bob", "v", fakeVideoStream("native-a"));
    const b = env.peers[0]!.receive("u-cara", "v", fakeVideoStream("native-b"));
    env.emitSig({
      op: "sig",
      t: "u",
      s: "srv",
      c: "voice",
      u: "u-bob",
      k: "v",
    });
    expect(useVoice.getState().remote["u-bob"]?.v).toBeUndefined();
    expect(useVoice.getState().remote["u-cara"]?.v).toBe(b.stream);
    expect(trackStopped(a.track)).toBe(false);
  });
  it("keeps a listening seat after microphone denial without publishing any source", async () => {
    const env = await joined({ media: false });
    await vi.waitFor(() => expect(env.errors).toHaveLength(1));
    expect(useVoice.getState().status).toBe("joined");
    expect(env.peers[0]!.closed).toBe(false);
    expect(env.peers[0]!.senderRows).toHaveLength(0);
    expect(env.mediaSent.some((f) => f.op === "produce")).toBe(false);
    const source = env.peers[0]!.receive(
      "u-bob",
      "v",
      fakeVideoStream("received"),
    );
    expect(useVoice.getState().remote["u-bob"]?.v).toBe(source.stream);
  });
  it("keeps the existing muted capture and Producer after a codec change is rejected", async () => {
    let rejectReplacement = false;
    const env = await joined({
      produceError: (kind) =>
        rejectReplacement && kind === "a"
          ? new Error("unsupported_codec")
          : undefined,
    });
    const sender = env.peers[0]!.sender("a")!;
    const track = sender.track;
    toggleMute();
    rejectReplacement = true;
    useMediaSettings.getState().patch({ economyMode: true, quality: "phone" });
    await vi.waitFor(() => expect(env.errors).toHaveLength(1));
    expect(env.peers[0]!.sender("a")).toBe(sender);
    expect(env.peers[0]!.audio).toBe(track);
    expect(track?.enabled).toBe(false);
    expect(trackStopped(track)).toBe(false);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(useVoice.getState().status).toBe("joined");
    expect(
      env.mediaSent.filter((f) => f.op === "produce").at(-1),
    ).toMatchObject({
      expectedOldProducerId: sender.producerId,
      epoch: sender.epoch,
    });
  });
  it("recovers send and receive ICE independently and cancels both deadlines on connection", async () => {
    const env = await joined();
    vi.useFakeTimers();
    const seat = env.peers[0]!;
    seat.setTransportState("send", "failed");
    seat.setTransportState("recv", "failed");
    seat.setTransportState("send", "failed");
    await vi.advanceTimersByTimeAsync(0);
    expect(seat.restartCalls).toEqual(["send", "recv"]);
    seat.setTransportState("send", "connected");
    seat.setTransportState("recv", "connected");
    await vi.advanceTimersByTimeAsync(15000);
    expect(env.peers).toHaveLength(1);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(useVoice.getState().status).toBe("joined");
  });
  it("bounds ICE recovery and rebuilds with the same capture and preferences", async () => {
    const env = await joined();
    const mic = env.peers[0]!.audio;
    toggleMute();
    vi.useFakeTimers();
    env.peers[0]!.setTransportState("recv", "failed");
    await vi.advanceTimersByTimeAsync(11000);
    expect(env.peers).toHaveLength(2);
    expect(env.peers[1]!.audio).toBe(mic);
    expect(mic?.enabled).toBe(false);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(env.peers[0]!.closed).toBe(true);
    expect(useVoice.getState().muted).toBe(true);
  });
  it("gates source audio on explicit Watch and keeps source volume independent while Deafen wins", async () => {
    const elements = playback();
    const env = await joined();
    const seat = env.peers[0]!;
    const audio = seat.receive("u-bob", "sa", fakeStream("looks-like-voice"));
    const video = seat.receive("u-bob", "s", fakeVideoStream("random-id"));
    expect(
      elements.filter((el) => el.attributes["data-source-audio"]),
    ).toHaveLength(0);
    expect(useVoice.getState().remote["u-bob"]?.s).toBeUndefined();
    toggleSourceWatch("u-bob", "s");
    const el = elements.find(
      (candidate) => candidate.attributes["data-source-audio"] === "s",
    )!;
    expect(el.srcObject).toBe(audio.stream);
    expect(useVoice.getState().remote["u-bob"]?.s).toBe(video.stream);
    useMediaSettings
      .getState()
      .patch({ sourceAudioVolume: 0.3, outputVolume: 0.8 });
    expect(el.volume).toBe(0.3);
    useMediaSettings
      .getState()
      .patch({ outputVolume: 0.1, sourceAudioMuted: true });
    expect(el.volume).toBe(0.3);
    expect(el.muted).toBe(true);
    toggleDeafen();
    expect(el.volume).toBe(0);
    toggleDeafen();
    expect(el.volume).toBe(0.3);
    expect(el.muted).toBe(true);
    toggleSourceWatch("u-bob", "s");
    expect(el.srcObject).toBeNull();
    expect(el.pause).toHaveBeenCalled();
    toggleSourceWatch("u-bob", "s");
    expect(
      elements.filter((candidate) => candidate.srcObject === audio.stream),
    ).toHaveLength(1);
  });
  it("plays every Watch room microphone once, deafens it, and accepts only selected Live audio", async () => {
    const elements = playback();
    const env = await joined();
    watchLive({ serverId: "srv", channelId: "stage", channelName: "Stage" });
    await vi.waitFor(() => expect(env.peers[1]?.started).toBe(true));
    const watch = env.peers[1]!;
    const a = watch.receive("u-bob", "a", fakeStream("native-mic-a"));
    const b = watch.receive("u-cara", "a", fakeStream("native-mic-b"));
    expect(elements.filter((el) => el.srcObject)).toHaveLength(2);
    for (const owner of ["u-self", "u-cara", "u-bob"])
      watch.receive(owner, "la", fakeStream(`native-live-${owner}`));
    const live = elements.filter(
      (el) => el.attributes["data-source-audio"] === "l",
    );
    expect(live).toHaveLength(1);
    expect(live[0]!.attributes["data-publisher"]).toBe("u-bob");
    useMediaSettings
      .getState()
      .patch({ outputVolume: 0.4, sourceAudioVolume: 0.7 });
    expect(
      elements
        .filter((el) => el.srcObject === a.stream || el.srcObject === b.stream)
        .every((el) => el.volume === 0.4),
    ).toBe(true);
    toggleDeafen();
    expect(
      elements.filter((el) => el.srcObject).every((el) => el.volume === 0),
    ).toBe(true);
    a.track.dispatchEvent(new Event("ended"));
    expect(elements[0]!.srcObject).toBeNull();
    stopWatching();
    expect(elements.every((el) => el.srcObject === null)).toBe(true);
    expect(env.getUserMediaCalls()).toBe(1);
    expect(env.getDisplayMediaCalls()).toBe(0);
  });
  it("retries blocked source playback on the user gesture and ignores late failure after Consumer close", async () => {
    const elements = playback();
    const env = await joined();
    toggleSourceWatch("u-bob", "s");
    const source = env.peers[0]!.receive("u-bob", "sa", fakeStream("source"));
    const el = elements.find(
      (candidate) => candidate.srcObject === source.stream,
    )!;
    el.play.mockRejectedValueOnce(new Error("NotAllowedError"));
    retryPlayback();
    await vi.waitFor(() =>
      expect(useVoice.getState().playbackBlocked).toBe(true),
    );
    retryPlayback();
    await vi.waitFor(() =>
      expect(useVoice.getState().playbackBlocked).toBe(false),
    );
    let reject!: (e: Error) => void;
    el.play.mockImplementationOnce(
      () =>
        new Promise((_resolve, no) => {
          reject = no;
        }),
    );
    retryPlayback();
    env.peers[0]!.removeReceived(source);
    reject(new Error("detached"));
    await Promise.resolve();
    await Promise.resolve();
    expect(el.srcObject).toBeNull();
    expect(useVoice.getState().playbackBlocked).toBe(false);
  });
  it("replays call audio when the page is shown again after an interruption", async () => {
    const elements = playback();
    let onVisibility: (() => void) | undefined;
    const page = {
      visibilityState: "visible",
      addEventListener: (type: string, listener: () => void) => {
        if (type === "visibilitychange") onVisibility = listener;
      },
      removeEventListener: () => undefined,
      querySelectorAll: () => [],
    };
    vi.stubGlobal("document", page);
    const env = await joined();
    toggleSourceWatch("u-bob", "s");
    const source = env.peers[0]!.receive("u-bob", "sa", fakeStream("source"));
    const el = elements.find(
      (candidate) => candidate.srcObject === source.stream,
    )!;
    el.play.mockClear();
    expect(onVisibility).toBeTypeOf("function");

    // A phone call or the lock screen hides the page and pauses the element.
    page.visibilityState = "hidden";
    onVisibility!();
    expect(el.play).not.toHaveBeenCalled();
    page.visibilityState = "visible";
    onVisibility!();
    expect(el.play).toHaveBeenCalledTimes(1);
    expect(useVoice.getState().playbackBlocked).toBe(false);

    // A browser that wants a gesture first raises the "Ton starten" recovery.
    el.play.mockRejectedValueOnce(
      Object.assign(new Error("gesture"), { name: "NotAllowedError" }),
    );
    onVisibility!();
    await vi.waitFor(() =>
      expect(useVoice.getState().playbackBlocked).toBe(true),
    );
    retryPlayback();
    await vi.waitFor(() =>
      expect(useVoice.getState().playbackBlocked).toBe(false),
    );

    leaveVoice();
    el.play.mockClear();
    onVisibility!();
    expect(el.play).not.toHaveBeenCalled();
    expect(useVoice.getState().playbackBlocked).toBe(false);
  });

  it("ends only source audio first and then stops both captures when the parent video ends", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const env = await joined();
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    const capture = useVoice.getState().localScreen!;
    capture.getAudioTracks()[0]!.dispatchEvent(new Event("ended"));
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeUndefined());
    expect(useVoice.getState().sourceAudio.s).toBe("ended");
    expect(useVoice.getState().localScreen).toBe(capture);
    expect(trackStopped(capture.getVideoTracks()[0])).toBe(false);
    capture.getVideoTracks()[0]!.dispatchEvent(new Event("ended"));
    expect(useVoice.getState().localScreen).toBeNull();
    expect(streamStopped(capture)).toBe(true);
    expect(trackStopped(env.peers[0]!.audio)).toBe(false);
  });
});

describe("display-source audio", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    vi.unstubAllGlobals();
    trackSeq = 0;
  });

  async function joined(options?: Parameters<typeof install>[0]) {
    const env = install(options);
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    return env;
  }

  function audioElements() {
    const elements: FakeAudio[] = [];
    class FakeAudio {
      autoplay = false;
      muted = false;
      volume = 1;
      srcObject: MediaStream | null = null;
      attributes: Record<string, string> = {};
      pause = vi.fn();
      play = vi.fn(async () => undefined);
      constructor() {
        elements.push(this);
      }
      setAttribute(key: string, value: string) {
        this.attributes[key] = value;
      }
    }
    vi.stubGlobal("Audio", FakeAudio);
    return elements;
  }

  it.each(["s", "l"] as const)(
    "captures %s video and browser audio once and publishes separate music, preserving the mic",
    async (kind) => {
      useMediaSettings
        .getState()
        .patch({ sourceAudioShare: "on", quality: "phone" });
      const elements = audioElements();
      const env = await joined();
      const mic = env.streams[0]?.getAudioTracks()[0];
      (kind === "s" ? toggleShare : toggleGoLive)();
      await vi.waitFor(() =>
        expect(
          env.mediaSent.some(
            (frame) =>
              frame.op === "produce" &&
              frame.k === (kind === "s" ? "sa" : "la"),
          ),
        ).toBe(true),
      );
      expect(env.getDisplayMediaCalls()).toBe(1);
      expect(env.lastDisplayMedia()?.audio).toEqual({
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
        channelCount: 2,
      });
      const capture =
        kind === "s"
          ? useVoice.getState().localScreen!
          : useVoice.getState().localLive!;
      const audio = capture.getAudioTracks()[0]!;
      const sourceSender = env.peers[0]?.senderRows.find(
        (sender) => sender.track === audio,
      );
      expect(
        sourceSender?.getParameters?.().encodings[0]?.maxBitrate,
      ).toBeUndefined();
      expect(
        env.peers[0]?.senderRows
          .find((sender) => sender.track === mic)
          ?.getParameters?.().encodings[0]?.maxBitrate,
      ).toBeUndefined();
      expect(
        (audio as MediaStreamTrack & { contentHint: string }).contentHint,
      ).toBe("music");
      const manifests = env.mediaSent.filter((frame) => frame.op === "produce");
      expect(manifests.slice(-2).map((frame) => frame.k)).toEqual([
        kind,
        kind === "s" ? "sa" : "la",
      ]);
      if (kind === "l")
        expect(
          manifests
            .slice(-2)
            .every(
              (frame) => frame.lc === "00000000-0000-0000-0000-000000000001",
            ),
        ).toBe(true);
      expect(
        elements.every((el) => !el.srcObject?.getTracks().includes(audio)),
      ).toBe(true);
      toggleMute();
      expect(mic?.enabled).toBe(false);
      expect(audio.enabled).toBe(true);
      leaveVoice();
      expect(streamStopped(capture)).toBe(true);
    },
  );

  it("continues video with an unavailable-audio status when the browser supplies no audio", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const env = await joined({
      displayStreamFor: () => fakeVideoStream("without-audio"),
    });
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBeTruthy(),
    );
    expect(useVoice.getState().sourceAudio.s).toBe("unavailable");
    expect(useVoice.getState().sharing).toBe(true);
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "sa"),
    ).toBe(false);
    expect(env.errors).toEqual([]);
  });

  it("preserves capture, subscriptions and source preferences on media reconnect without a new picker", async () => {
    useMediaSettings.getState().patch({
      sourceAudioShare: "on",
      sourceAudioVolume: 0.4,
      sourceAudioMuted: true,
    });
    const env = await joined();
    toggleSourceWatch("u-bob", "s");
    toggleShare();
    await vi.waitFor(() =>
      expect(
        env.mediaSent.some(
          (frame) => frame.op === "produce" && frame.k === "sa",
        ),
      ).toBe(true),
    );
    const capture = useVoice.getState().localScreen!;
    env.closeMedia();
    await vi.waitFor(() => expect(env.peers.length).toBe(2));
    await vi.waitFor(() =>
      expect(
        env.peers[1]?.senderRows.some(
          (sender) => sender.track === capture.getAudioTracks()[0],
        ),
      ).toBe(true),
    );
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(useVoice.getState().localScreen).toBe(capture);
    expect(streamStopped(capture)).toBe(false);
    expect(
      env.mediaSent.filter((frame) => frame.op === "w" && frame.on),
    ).toHaveLength(2);
    expect(useMediaSettings.getState()).toMatchObject({
      sourceAudioShare: "on",
      sourceAudioVolume: 0.4,
      sourceAudioMuted: true,
    });
  });

  it("cleans both source tracks after rejected source-audio production while keeping the microphone", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const capture = fakeVideoStream("rejected-source", true);
    const env = await joined({
      displayStreamFor: () => capture,
      produceError: (kind) =>
        kind === "sa" ? new Error("negotiation_failed") : undefined,
    });
    toggleShare();
    await vi.waitFor(() =>
      expect(
        env.mediaSent.some(
          (frame) => frame.op === "produce" && frame.k === "sa",
        ),
      ).toBe(true),
    );
    const mic = env.streams[0]!.getAudioTracks()[0];
    await vi.waitFor(() => expect(useVoice.getState().localScreen).toBeNull());
    expect(streamStopped(capture)).toBe(true);
    expect(trackStopped(mic)).toBe(false);
    expect(useVoice.getState().status).toBe("joined");
    expect(useVoice.getState().sourceAudio.s).toBe("off");
  });

  it("falls back on the granted display track without reopening capture", async () => {
    useMediaSettings
      .getState()
      .patch({ screenProfile: "detail", sourceAudioShare: "on" });
    const stream = fakeVideoStream("fallback", true);
    const rejection = new Error("size");
    rejection.name = "OverconstrainedError";
    const apply = vi
      .fn()
      .mockRejectedValueOnce(rejection)
      .mockResolvedValue(undefined);
    stream.getVideoTracks()[0]!.applyConstraints = apply;
    const env = await joined({ displayStreamFor: () => stream });
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().localScreen).toBe(stream),
    );
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(apply.mock.calls.map((call) => call[0])).toEqual([
      videoConstraintsFor("screen", "detail"),
      videoConstraintsFor("screen", "balanced"),
    ]);
    expect(streamStopped(stream)).toBe(false);
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
  });

  it("stops a granted video/audio capture immediately when leaving during profile application", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const stream = fakeVideoStream("pending-profile", true);
    const gate = deferred();
    const apply = vi.fn(() => gate.promise);
    stream.getVideoTracks()[0]!.applyConstraints = apply;
    const env = await joined({ displayStreamFor: () => stream });
    toggleShare();
    await vi.waitFor(() => expect(apply).toHaveBeenCalled());
    leaveVoice();
    expect(streamStopped(stream)).toBe(true);
    gate.resolve();
    await Promise.resolve();
    await Promise.resolve();
    expect(useVoice.getState().localScreen).toBeNull();
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "sa"),
    ).toBe(false);
  });

  /** "p" and "u" the gateway heard for one source kind. */
  const announced = (env: ReturnType<typeof install>, kind: TrackKind) =>
    env.sent.flatMap((frame) =>
      frame.op === "sig" && "k" in frame && frame.k === kind ? [frame.t] : [],
    );

  it.each(["s", "l"] as const)(
    "asks the browser for no %s sound while the user has not chosen it",
    async (kind) => {
      const env = await joined({
        displayStreamFor: (_index, constraints) =>
          fakeVideoStream("as-asked", Boolean(constraints.audio)),
      });
      (kind === "s" ? toggleShare : toggleGoLive)();
      await vi.waitFor(() => expect(env.peers[0]?.sender(kind)).toBeTruthy());
      expect(env.lastDisplayMedia()?.audio).toBe(false);
      expect(useVoice.getState().sourceAudio[kind]).toBe("off");
      expect(env.peers[0]?.sender(kind === "s" ? "sa" : "la")).toBeUndefined();
    },
  );

  it("takes the sound out of a running share at once; a browser adds none without a new picker", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const env = await joined();
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    const capture = useVoice.getState().localScreen!;
    const video = env.peers[0]!.sender("s");

    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeUndefined());
    expect(trackStopped(capture.getAudioTracks()[0])).toBe(true);
    expect(trackStopped(capture.getVideoTracks()[0])).toBe(false);
    // Switched off is not "ended": no notice about lost sound.
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(useVoice.getState().localScreen).toBe(capture);
    expect(env.peers[0]!.sender("s")).toBe(video);
    expect(announced(env, "sa")).toEqual(["p", "u"]);
    expect(announced(env, "s")).toEqual(["p"]);

    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await Promise.resolve();
    await Promise.resolve();
    expect(env.getDisplayMediaCalls()).toBe(1);
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(env.peers[0]?.sender("sa")).toBeUndefined();
    expect(env.errors).toEqual([]);
  });

  it("forgets that the browser gave no sound once the user switches it off", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await joined({ displayStreamFor: () => fakeVideoStream("without-audio") });
    toggleShare();
    await vi.waitFor(() =>
      expect(useVoice.getState().sourceAudio.s).toBe("unavailable"),
    );
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(useVoice.getState().sharing).toBe(true);
  });

  it("does not announce sound that was switched off while its publish was under way", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const answer = deferred();
    const env = await joined({
      gateProduce: (kind) => (kind === "sa" ? answer.promise : undefined),
    });
    toggleShare();
    await vi.waitFor(() =>
      expect(
        env.mediaSent.some(
          (frame) => frame.op === "produce" && frame.k === "sa",
        ),
      ).toBe(true),
    );
    const capture = useVoice.getState().localScreen!;
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    expect(trackStopped(capture.getAudioTracks()[0])).toBe(true);
    expect(announced(env, "sa")).toEqual(["u"]);

    // The producer that arrives after all is closed, not announced.
    answer.resolve();
    await vi.waitFor(() =>
      expect(env.mediaSent.some((frame) => frame.op === "closeProducer")).toBe(
        true,
      ),
    );
    expect(env.peers[0]?.sender("sa")).toBeUndefined();
    expect(announced(env, "sa")).toEqual(["u"]);
    expect(useVoice.getState().participants["u-self"]?.pubs).toEqual([
      "a",
      "s",
    ]);
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    // The video is not part of it.
    expect(env.peers[0]?.sender("s")).toBeTruthy();
    expect(trackStopped(capture.getVideoTracks()[0])).toBe(false);
    expect(useVoice.getState().sharing).toBe(true);
    expect(env.errors).toEqual([]);
  });

  it("shares no sound that was switched off while the picker was open", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const picker = deferred();
    const stream = fakeVideoStream("picked-late", true);
    const env = await joined({
      holdDisplay: picker.promise,
      displayStreamFor: () => stream,
    });
    toggleShare();
    await Promise.resolve();
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    picker.resolve();
    await vi.waitFor(() => expect(env.peers[0]?.sender("s")).toBeTruthy());
    expect(env.lastDisplayMedia()?.audio).toBeTruthy();
    expect(trackStopped(stream.getAudioTracks()[0])).toBe(true);
    expect(trackStopped(stream.getVideoTracks()[0])).toBe(false);
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(
      env.mediaSent.some((frame) => frame.op === "produce" && frame.k === "sa"),
    ).toBe(false);
  });
});

describe("stream sound in the desktop app", () => {
  afterEach(() => {
    resetVoiceForTests();
    resetVoiceRoster();
    resetMediaSettingsForTests();
    setNativeBridgeForTests(undefined);
    vi.restoreAllMocks();
    vi.useRealTimers();
    trackSeq = 0;
  });

  /** Released 0.5.x: no feature list, and "every application" includes the
   * app's own playout. */
  const APP_05 = { abi: 7, version: "0.5.2", platform: "linux" };
  /** A v0.6 build whose core still captures itself. */
  const APP_06_CAPTURES_ITSELF = {
    abi: 8,
    version: "0.6.0",
    platform: "linux",
    features: ["screen", "camera", "app-audio", "video-frames"],
  };
  const APP_06 = {
    ...APP_06_CAPTURES_ITSELF,
    features: [...APP_06_CAPTURES_ITSELF.features, "app-audio-excludes-self"],
  };

  /** The desktop app as the session meets it: `media_info`, the desktop's
   * picker and application sound. Every other command answers null. */
  function desktopApp(info: Record<string, unknown>) {
    const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
    const sounds: number[] = [];
    let next = 0;
    const app = {
      /** The core's reason for giving no application sound. */
      soundError: null as string | null,
      /** Playback streams the chosen application has. */
      streams: 1,
      /** Application-sound sources handed out, oldest first. */
      sounds,
      called: (command: string) =>
        calls.filter((call) => call.command === command).map((c) => c.args),
    };
    setNativeBridgeForTests({
      async invoke<T>(command: string, args: Record<string, unknown> = {}) {
        calls.push({ command, args });
        const answer = (): unknown => {
          switch (command) {
            case "media_info":
              return info;
            case "media_source_screen":
            case "media_source_microphone":
              return ++next;
            case "media_source_app_audio":
              // The app rejects with the core's message as a plain string.
              if (app.soundError) return Promise.reject(app.soundError);
              sounds.push(++next);
              return next;
            case "media_source_state":
              return sounds.includes(args.source as number)
                ? { state: "live", streams: app.streams, frames: 0 }
                : { state: "live", width: 1920, height: 1080 };
            default:
              return null;
          }
        };
        return (await answer()) as T;
      },
      async channel() {
        return null;
      },
    });
    return app;
  }

  async function joined(
    info: Record<string, unknown>,
    options?: Parameters<typeof install>[0],
  ) {
    const app = desktopApp(info);
    const env = install({ ...options, nativeDisplay: true });
    joinVoice({ serverId: "srv", channelId: "voice", channelName: "Lounge" });
    await vi.waitFor(() => expect(env.peers[0]?.audio).toBeTruthy());
    return { env, app };
  }

  const soundOf = (kind: "s" | "l") => (kind === "s" ? "sa" : "la");
  const produced = (env: ReturnType<typeof install>, kind: TrackKind) =>
    env.mediaSent.filter((frame) => frame.op === "produce" && frame.k === kind);

  it.each([
    ["0.5.x app, nothing chosen", APP_05, "auto", "", null],
    ["0.5.x app, switched on", APP_05, "on", "", ""],
    // 0.5.x named applications by their name; the fixed core still takes it.
    ["0.5.x app, one application", APP_05, "on", "Firefox", "Firefox"],
    [
      "v0.6 app that captures itself, nothing chosen",
      APP_06_CAPTURES_ITSELF,
      "auto",
      "",
      null,
    ],
    ["v0.6 app, nothing chosen", APP_06, "auto", "", ""],
    ["v0.6 app, an old application name", APP_06, "auto", "Spotify", "Spotify"],
    ["v0.6 app, one application", APP_06, "on", "spotify", "spotify"],
    ["v0.6 app, switched off", APP_06, "off", "", null],
  ] as const)(
    "%s: what screen share and Go Live ask the app for",
    async (_name, info, sourceAudioShare, sourceAudioApp, expected) => {
      useMediaSettings.getState().patch({ sourceAudioShare, sourceAudioApp });
      const { env, app } = await joined(info);
      for (const kind of ["s", "l"] as const) {
        (kind === "s" ? toggleShare : toggleGoLive)();
        await vi.waitFor(() => expect(env.peers[0]?.sender(kind)).toBeTruthy());
        if (expected !== null)
          await vi.waitFor(() =>
            expect(env.peers[0]?.sender(soundOf(kind))).toBeTruthy(),
          );
      }
      // One picker each, and never a browser capture inside the app.
      expect(app.called("media_source_screen")).toHaveLength(2);
      expect(env.getDisplayMediaCalls()).toBe(0);
      expect(app.called("media_source_app_audio")).toEqual(
        expected === null
          ? []
          : [{ options: { app: expected } }, { options: { app: expected } }],
      );
      for (const kind of ["s", "l"] as const) {
        const video = env.peers[0]!.sender(kind)!;
        const sound = env.peers[0]!.sender(soundOf(kind));
        const capture =
          kind === "s"
            ? useVoice.getState().localScreen!
            : useVoice.getState().localLive!;
        if (expected === null) {
          expect(sound).toBeUndefined();
          expect(capture.getAudioTracks()).toEqual([]);
          expect(useVoice.getState().sourceAudio[kind]).toBe("off");
          continue;
        }
        expect(sound?.track).toBe(capture.getAudioTracks()[0]);
        expect(produced(env, soundOf(kind))).toEqual([
          expect.objectContaining({
            parent: video.producerId,
            epoch: video.epoch,
          }),
        ]);
        expect(useVoice.getState().sourceAudio[kind]).toBe("sharing");
      }
      expect(useVoice.getState().sourceAudioNote).toEqual({ s: null, l: null });
      expect(env.errors).toEqual([]);
    },
  );

  it("adds and removes application sound while the share runs, without a picker or a new video", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    const { env, app } = await joined(APP_06);
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("s")).toBeTruthy());
    const capture = useVoice.getState().localScreen!;
    const video = env.peers[0]!.sender("s")!;
    expect(app.called("media_source_app_audio")).toEqual([]);

    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    expect(app.called("media_source_app_audio")).toEqual([
      { options: { app: "" } },
    ]);
    expect(app.called("media_source_screen")).toHaveLength(1);
    const [sound] = capture.getAudioTracks();
    expect(env.peers[0]!.sender("sa")?.track).toBe(sound);
    expect((sound as unknown as { contentHint: string }).contentHint).toBe(
      "music",
    );
    expect(produced(env, "sa")).toEqual([
      expect.objectContaining({
        parent: video.producerId,
        epoch: video.epoch,
      }),
    ]);
    await vi.waitFor(() =>
      expect(useVoice.getState().participants["u-self"]?.pubs).toContain("sa"),
    );
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");

    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeUndefined());
    expect(sound.readyState).toBe("ended");
    expect(app.called("media_source_close")).toEqual([
      { source: app.sounds[0] },
    ]);
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(useVoice.getState().participants["u-self"]?.pubs).not.toContain(
      "sa",
    );

    // The video never moved.
    expect(useVoice.getState().localScreen).toBe(capture);
    expect(capture.getVideoTracks()[0]!.readyState).toBe("live");
    expect(env.peers[0]!.sender("s")).toBe(video);
    expect(produced(env, "s")).toHaveLength(1);
    expect(app.called("media_source_screen")).toHaveLength(1);
    expect(env.errors).toEqual([]);
  });

  it("takes the sound out of a running share when the switch keeps no off", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    const { env, app } = await joined(APP_05);
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    const video = env.peers[0]!.sender("s");
    // Off is what this app does unasked: the switch stores no choice.
    expect(sourceAudioChoice(false)).toBe("auto");
    useMediaSettings
      .getState()
      .patch({ sourceAudioShare: sourceAudioChoice(false) });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeUndefined());
    expect(app.called("media_source_close")).toEqual([
      { source: app.sounds[0] },
    ]);
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(env.peers[0]!.sender("s")).toBe(video);
  });

  it("adds application sound to a running Go Live under its claim", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    const { env, app } = await joined(APP_06);
    toggleGoLive();
    await vi.waitFor(() => expect(env.peers[0]?.sender("l")).toBeTruthy());
    const video = env.peers[0]!.sender("l")!;
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("la")).toBeTruthy());
    expect(app.called("media_source_screen")).toHaveLength(1);
    expect(produced(env, "la")).toEqual([
      expect.objectContaining({
        parent: video.producerId,
        epoch: video.epoch,
        lc: "00000000-0000-0000-0000-000000000001",
      }),
    ]);
    expect(useVoice.getState().sourceAudio).toEqual({ s: "off", l: "sharing" });
    expect(liveClaimFrames(env.sent)).toEqual(["p"]);
  });

  it("changes the application of a running share to the one just chosen", async () => {
    const { env, app } = await joined(APP_06);
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    const capture = useVoice.getState().localScreen!;
    const [first] = capture.getAudioTracks();

    useMediaSettings.getState().patch({ sourceAudioApp: "spotify" });
    await vi.waitFor(() =>
      expect(app.called("media_source_app_audio")).toEqual([
        { options: { app: "" } },
        { options: { app: "spotify" } },
      ]),
    );
    await vi.waitFor(() => {
      const sender = env.peers[0]?.sender("sa");
      expect(sender).toBeTruthy();
      expect(sender!.track).not.toBe(first);
    });
    expect(first.readyState).toBe("ended");
    expect(app.called("media_source_close")).toEqual([
      { source: app.sounds[0] },
    ]);
    // The share holds the new sound only; the old track is not kept around.
    expect(capture.getAudioTracks()).toEqual([
      env.peers[0]!.sender("sa")!.track,
    ]);
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
    expect(app.called("media_source_screen")).toHaveLength(1);
  });

  it("says why the app captured no sound and tries again when asked", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const { env, app } = await joined(APP_06);
    app.soundError = "cannot connect to PipeWire";
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("s")).toBeTruthy());
    expect(useVoice.getState().sourceAudio.s).toBe("unavailable");
    expect(useVoice.getState().sourceAudioNote.s).toEqual({
      failed: "cannot connect to PipeWire",
    });
    expect(useVoice.getState().sharing).toBe(true);
    expect(env.peers[0]?.sender("sa")).toBeUndefined();
    // Reported next to the control, not as a passing toast.
    expect(env.errors).toEqual([]);
    expect(warn).toHaveBeenCalledWith(
      "[gelabber:voice]",
      expect.objectContaining({
        step: "source-audio",
        detail: "cannot connect to PipeWire",
      }),
    );

    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    expect(useVoice.getState().sourceAudio.s).toBe("off");
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();

    // Still failing: the reason comes back with the next attempt.
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() =>
      expect(useVoice.getState().sourceAudio.s).toBe("unavailable"),
    );
    expect(useVoice.getState().sourceAudioNote.s).toEqual({
      failed: "cannot connect to PipeWire",
    });

    app.soundError = null;
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
    expect(app.called("media_source_screen")).toHaveLength(1);
  });

  it("keeps the video when sound added to a running share is refused", async () => {
    vi.spyOn(console, "warn").mockImplementation(() => undefined);
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    const { env, app } = await joined(APP_06, {
      produceError: (kind) =>
        kind === "sa" ? new Error("negotiation_failed") : undefined,
    });
    toggleShare();
    await vi.waitFor(() => expect(env.peers[0]?.sender("s")).toBeTruthy());
    const capture = useVoice.getState().localScreen!;
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() =>
      expect(useVoice.getState().sourceAudio.s).toBe("unavailable"),
    );
    expect(useVoice.getState().sourceAudioNote.s).toEqual({
      failed: expect.stringContaining("negotiation_failed"),
    });
    expect(useVoice.getState().localScreen).toBe(capture);
    expect(capture.getVideoTracks()[0]!.readyState).toBe("live");
    expect(env.peers[0]?.sender("s")).toBeTruthy();
    expect(env.peers[0]?.sender("sa")).toBeUndefined();
    // The capture it started for nothing is released.
    expect(app.called("media_source_close")).toEqual([
      { source: app.sounds[0] },
    ]);
    expect(
      capture.getAudioTracks().every((track) => track.readyState === "ended"),
    ).toBe(true);
  });

  it.each([
    ["s", "before its publish"],
    ["s", "during its publish"],
    ["l", "before its publish"],
    ["l", "during its publish"],
  ] as const)(
    "keeps %s sound switched on as the media connection drops %s, and sends it with the new one",
    async (kind, moment) => {
      const warn = vi
        .spyOn(console, "warn")
        .mockImplementation(() => undefined);
      useMediaSettings.getState().patch({ sourceAudioShare: "off" });
      const lost = deferred();
      let held = false;
      const { env, app } = await joined(APP_06, {
        // The first answer never comes: the socket dies under it.
        gateProduce: (asked) => {
          if (moment !== "during its publish" || held) return undefined;
          if (asked !== soundOf(kind)) return undefined;
          held = true;
          return lost.promise;
        },
      });
      (kind === "s" ? toggleShare : toggleGoLive)();
      await vi.waitFor(() => expect(env.peers[0]?.sender(kind)).toBeTruthy());
      const capture =
        kind === "s"
          ? useVoice.getState().localScreen!
          : useVoice.getState().localLive!;

      // The seat holds the dead connection until its first rebuild.
      if (moment === "before its publish") env.closeMedia();
      useMediaSettings.getState().patch({ sourceAudioShare: "on" });
      await vi.waitFor(() =>
        expect(
          env.peers[0]!.publicationInputs.some(
            (input) => input.kind === soundOf(kind),
          ),
        ).toBe(true),
      );
      if (moment === "during its publish") {
        expect(produced(env, soundOf(kind))).toHaveLength(1);
        env.closeMedia();
      }

      await vi.waitFor(() =>
        expect(env.peers[1]?.sender(soundOf(kind))).toBeTruthy(),
      );
      const [sound] = capture.getAudioTracks();
      expect(sound.readyState).toBe("live");
      expect(env.peers[1]!.sender(soundOf(kind))!.track).toBe(sound);
      expect(produced(env, soundOf(kind)).at(-1)).toMatchObject({
        parent: env.peers[1]!.sender(kind)!.producerId,
      });
      await vi.waitFor(() =>
        expect(useVoice.getState().participants["u-self"]?.pubs).toContain(
          soundOf(kind),
        ),
      );
      expect(useVoice.getState().sourceAudio[kind]).toBe("sharing");
      expect(useVoice.getState().sourceAudioNote[kind]).toBeNull();
      // One capture: nothing was given up and asked for again.
      expect(app.called("media_source_app_audio")).toHaveLength(1);
      expect(app.called("media_source_close")).toEqual([]);
      expect(app.called("media_source_screen")).toHaveLength(1);
      expect(capture.getVideoTracks()[0]!.readyState).toBe("live");
      expect(warn).not.toHaveBeenCalledWith(
        "[gelabber:voice]",
        expect.objectContaining({ step: "source-audio" }),
      );
      expect(env.errors).toEqual([]);
    },
  );

  it("leaves the sound switched on again alone when the one before it arrives late", async () => {
    const answer = deferred();
    let held = false;
    const { env, app } = await joined(APP_06, {
      gateProduce: (kind) => {
        if (kind !== "sa" || held) return undefined;
        held = true;
        return answer.promise;
      },
    });
    toggleShare();
    await vi.waitFor(() => expect(produced(env, "sa")).toHaveLength(1));
    const capture = useVoice.getState().localScreen!;
    const [first] = capture.getAudioTracks();

    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    expect(first.readyState).toBe("ended");
    useMediaSettings.getState().patch({ sourceAudioShare: "on" });
    await vi.waitFor(() => expect(env.peers[0]?.sender("sa")).toBeTruthy());
    const second = env.peers[0]!.sender("sa")!;
    expect(second.track).not.toBe(first);
    await vi.waitFor(() =>
      expect(useVoice.getState().participants["u-self"]?.pubs).toContain("sa"),
    );

    // Microphone, video and both sounds: the late one is through as well.
    answer.resolve();
    await vi.waitFor(() => expect(env.peers[0]!.tracks).toBe(4));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(env.peers[0]!.sender("sa")).toBe(second);
    expect(second.track?.readyState).toBe("live");
    expect(
      env.mediaSent.some(
        (frame) =>
          frame.op === "closeProducer" &&
          frame.producerId === second.producerId,
      ),
    ).toBe(false);
    expect(
      env.sent.flatMap((frame) =>
        frame.op === "sig" && "k" in frame && frame.k === "sa" ? [frame.t] : [],
      ),
    ).toEqual(["u", "p"]);
    expect(useVoice.getState().participants["u-self"]?.pubs).toContain("sa");
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
    expect(app.called("media_source_close")).toEqual([
      { source: app.sounds[0] },
    ]);
    expect(env.errors).toEqual([]);
  });

  it("tells when the chosen application plays nothing, not for a short pause", async () => {
    useMediaSettings
      .getState()
      .patch({ sourceAudioShare: "on", sourceAudioApp: "spotify" });
    const { env, app } = await joined(APP_06);
    app.streams = 0;
    vi.useFakeTimers();
    toggleShare();
    await vi.advanceTimersByTimeAsync(0);
    expect(env.peers[0]?.sender("sa")).toBeTruthy();
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
    await vi.advanceTimersByTimeAsync(2_000);
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(useVoice.getState().sourceAudioNote.s).toEqual({
      silent: "spotify",
    });
    // Silence is still a running share.
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
    expect(env.peers[0]?.sender("sa")).toBeTruthy();

    app.streams = 1;
    await vi.advanceTimersByTimeAsync(1_000);
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
    app.streams = 0;
    await vi.advanceTimersByTimeAsync(2_000);
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();

    // Stopped: nobody asks the gone source any more.
    toggleShare();
    const asked = app.called("media_source_state").length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(app.called("media_source_state")).toHaveLength(asked);
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
  });

  it("takes the silence of every application for what it is", async () => {
    const { env, app } = await joined(APP_06);
    app.streams = 0;
    vi.useFakeTimers();
    toggleShare();
    await vi.advanceTimersByTimeAsync(0);
    expect(env.peers[0]?.sender("sa")).toBeTruthy();
    await vi.advanceTimersByTimeAsync(10_000);
    expect(useVoice.getState().sourceAudio.s).toBe("sharing");
    expect(useVoice.getState().sourceAudioNote.s).toBeNull();
    expect(
      app
        .called("media_source_state")
        .some((args) => args.source === app.sounds[0]),
    ).toBe(false);
  });
});
