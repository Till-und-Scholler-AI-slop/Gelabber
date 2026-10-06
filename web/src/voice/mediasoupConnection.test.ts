import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Device } from "mediasoup-client";
import type {
  ConsumerOptions,
  ProducerCodecOptions,
  ProducerOptions,
  RtpCapabilities,
  RtpParameters,
  TransportOptions,
} from "mediasoup-client/types";
import {
  MediasoupConnection,
  type MediaPublication,
  type ReceivedSource,
  type RtpSenderParameters,
} from "./mediasoupConnection.ts";
import type {
  ConsumerAnnouncement,
  MediaMethod,
  MediaRequest,
  MediaRequests,
} from "./media.ts";

const GENERATION = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const EPOCH = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
const NEXT_EPOCH = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const OWNER = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const capabilities: RtpCapabilities = {
  codecs: [
    {
      kind: "audio",
      mimeType: "audio/opus",
      preferredPayloadType: 111,
      clockRate: 48000,
      channels: 2,
    },
    {
      kind: "video",
      mimeType: "video/VP8",
      preferredPayloadType: 96,
      clockRate: 90000,
    },
  ],
};
const rtp: RtpParameters = {
  codecs: [
    { mimeType: "audio/opus", payloadType: 111, clockRate: 48000, channels: 2 },
  ],
  encodings: [{ ssrc: 17 }],
};
const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
};
const tick = async () => {
  for (let count = 0; count < 15; count += 1) await Promise.resolve();
};
async function foreignNativeOperationError(): Promise<unknown> {
  // Tests use the genuine Node VM without adding Node types to the browser tsconfig.
  const modulePath: string = "node:vm";
  const vm: { runInNewContext: (source: string) => unknown } = await import(
    modulePath
  );
  return vm.runInNewContext(
    "Object.assign(new Error('setRemoteDescription rejected'), {name: 'OperationError'})",
  );
}

class Capture {
  enabled = true;
  readyState = "live";
  stop = vi.fn(() => {
    this.readyState = "ended";
  });
  constructor(
    readonly id: string,
    readonly kind: "audio" | "video",
  ) {}
  getSettings() {
    return { height: this.kind === "video" ? 720 : undefined };
  }
  native() {
    return this as unknown as MediaStreamTrack;
  }
}
class NativeProducer {
  closed = false;
  paused = false;
  private parameters: RtpSenderParameters = {
    transactionId: "native-transaction",
    encodings: [{ maxBitrate: 72000, maxFramerate: 30 }],
  };
  rtpParameters = rtp;
  getStats = vi.fn(async () => new Map());
  rtpSender = {
    getParameters: () => structuredClone(this.parameters),
    setParameters: vi.fn(async (next: RtpSenderParameters) => {
      this.parameters = structuredClone(next);
    }),
  };
  constructor(
    readonly id: string,
    public track: MediaStreamTrack | null,
    private readonly stopTracks: boolean,
  ) {}
  close = vi.fn(() => {
    this.closed = true;
    if (this.stopTracks) this.track?.stop();
  });
  pause = vi.fn(() => {
    this.paused = true;
  });
  resume = vi.fn(() => {
    this.paused = false;
  });
  replaceTrack = vi.fn(
    async ({ track }: { track: MediaStreamTrack | null }) => {
      this.track = track;
    },
  );
  on = vi.fn();
}
class NativeConsumer {
  closed = false;
  paused = false;
  readonly track = new Capture("received", "audio").native();
  rtpReceiver = { getStats: vi.fn(async () => new Map()) };
  constructor(readonly id: string) {}
  close = vi.fn(() => {
    this.closed = true;
  });
  pause = vi.fn(() => {
    this.paused = true;
  });
  resume = vi.fn(() => {
    this.paused = false;
  });
  getStats = vi.fn(async () => new Map());
}
type Handler = (...args: unknown[]) => void;
class PublicTransport {
  connectionState = "connected";
  closed = false;
  handlers = new Map<string, Handler>();
  producers: NativeProducer[] = [];
  consumers: NativeConsumer[] = [];
  productionErrors: Error[] = [];
  failNativeSendOnce = false;
  nativeSendTainted = false;
  consumeGate: ReturnType<typeof deferred<NativeConsumer>> | null = null;
  constructor(
    readonly id: string,
    private readonly trace: string[],
  ) {}
  on(name: string, handler: Handler) {
    this.handlers.set(name, handler);
  }
  close = vi.fn(() => {
    this.trace.push("transport-close:" + this.id);
    this.closed = true;
    for (const producer of this.producers) producer.close();
    for (const consumer of this.consumers) consumer.close();
  });
  produce = vi.fn(async (options: ProducerOptions) => {
    // A genuine failed SDK send leaves its shared SDP handler unusable. The
    // public transport must be replaced; repeating produce cannot repair it.
    if (this.failNativeSendOnce || this.nativeSendTainted) {
      this.failNativeSendOnce = false;
      this.nativeSendTainted = true;
      throw new DOMException("setRemoteDescription rejected", "OperationError");
    }
    const error = this.productionErrors.shift();
    if (error) throw error;
    const id = await new Promise<string>((resolve, reject) => {
      this.handlers.get("produce")?.(
        { rtpParameters: rtp, appData: options.appData },
        ({ id }: { id: string }) => resolve(id),
        reject,
      );
    });
    const producer = new NativeProducer(
      id,
      options.track ?? null,
      options.stopTracks ?? true,
    );
    this.producers.push(producer);
    return producer;
  });
  consume = vi.fn(async (options: ConsumerOptions) => {
    this.trace.push("consume:" + options.id);
    const consumer = this.consumeGate
      ? await this.consumeGate.promise
      : new NativeConsumer(options.id);
    this.consumers.push(consumer);
    return consumer;
  });
  restartIce = vi.fn(async () => {});
  getStats = vi.fn(async () => new Map());
}
class PublicDevice {
  handlerName = "Chrome111";
  recvRtpCapabilities = capabilities;
  sendRtpCapabilities = capabilities;
  transports: PublicTransport[] = [];
  configureSend: ((transport: PublicTransport) => void) | null = null;
  load = vi.fn(async () => {});
  canProduce = vi.fn(() => true);
  constructor(readonly trace: string[]) {}
  createSendTransport(options: TransportOptions) {
    const transport = this.create(options);
    this.configureSend?.(transport);
    return transport;
  }
  createRecvTransport(options: TransportOptions) {
    return this.create(options);
  }
  private create(options: TransportOptions) {
    const transport = new PublicTransport(options.id, this.trace);
    this.transports.push(transport);
    return transport;
  }
}
type Rpc = { method: MediaMethod; data: unknown; deadlineEpochMs?: number };
function fixture(role: "voice" | "watch" = "voice", sdkTimeoutMs?: number) {
  const trace: string[] = [],
    requests: Rpc[] = [],
    attached: ReceivedSource[] = [];
  const device = new PublicDevice(trace);
  let transportNumber = 0,
    producerNumber = 0;
  let codecOptions: ProducerCodecOptions = { opusDtx: false, opusStereo: true };
  let intercept:
    | ((method: MediaMethod, data: unknown) => Promise<unknown> | undefined)
    | undefined;
  const request = vi.fn(
    async (method: MediaMethod, data: unknown, deadlineEpochMs?: number) => {
      requests.push({ method, data, deadlineEpochMs });
      trace.push("rpc:" + method);
      const overridden = intercept?.(method, data);
      if (overridden) return overridden;
      if (method === "transport")
        return {
          id: `${role}-${++transportNumber}`,
          iceParameters: { usernameFragment: "u", password: "p" },
          iceCandidates: [],
          dtlsParameters: { fingerprints: [] },
        };
      if (method === "produce")
        return { producerId: "producer-" + ++producerNumber };
      if (method === "restartIce")
        return {
          iceParameters: { usernameFragment: "next", password: "next" },
        };
      return {};
    },
  );
  const onConsumer = vi.fn((source: ReceivedSource) => {
    trace.push("attach:" + source.consumerId);
    attached.push(source);
  });
  const onError = vi.fn(),
    onConsumerClosed = vi.fn(),
    onTransportState = vi.fn();
  const connection = new MediasoupConnection({
    role,
    sdkTimeoutMs,
    generation: GENERATION,
    iceServers: [],
    request: request as MediaRequest,
    deviceFactory: async () => device as unknown as Device,
    codecOptions: () => codecOptions,
    onConsumer,
    onConsumerClosed,
    onError,
    onTransportState,
  });
  active.add(connection);
  return {
    connection,
    device,
    requests,
    trace,
    attached,
    onConsumer,
    onConsumerClosed,
    onError,
    onTransportState,
    setCodec: (options: ProducerCodecOptions) => {
      codecOptions = options;
    },
    intercept: (handler: typeof intercept) => {
      intercept = handler;
    },
    calls: <K extends MediaMethod>(method: K) =>
      requests
        .filter((rpc) => rpc.method === method)
        .map((rpc) => rpc.data as MediaRequests[K]),
    start: () => connection.start(capabilities),
  };
}
const active = new Set<MediasoupConnection>();
function publication(
  kind: MediaPublication["kind"],
  capture: Capture,
  extras: Partial<MediaPublication> = {},
): MediaPublication {
  return {
    kind,
    track: capture.native(),
    streamId: "capture-stream",
    epoch: EPOCH,
    ...extras,
  };
}
function announcement(
  extras: Partial<ConsumerAnnouncement> = {},
): ConsumerAnnouncement {
  return {
    op: "consumer",
    consumerId: "consumer-1",
    producerId: "remote-producer",
    owner: OWNER,
    k: "a",
    epoch: EPOCH,
    generation: GENERATION,
    kind: "audio",
    rtpParameters: rtp,
    paused: true,
    ...extras,
  };
}

beforeEach(() => {
  vi.stubGlobal(
    "MediaStream",
    class {
      constructor(readonly tracks: MediaStreamTrack[]) {}
      getTracks() {
        return this.tracks;
      }
    },
  );
});

describe("bounded live publication recovery through the public SDK", () => {
  it("bounds repeated live_busy failures without losing current captures, epochs, or audio parents", async () => {
    vi.useFakeTimers();
    const f = fixture();
    await f.start();
    let originalCurrent = true;
    const originalDeadline = Date.now() + 10_000;
    const captures = {
      a: new Capture("mic", "audio"),
      v: new Capture("camera", "video"),
      s: new Capture("screen", "video"),
      sa: new Capture("screen-audio", "audio"),
    };
    for (const kind of ["a", "v", "s", "sa"] as const) {
      await f.connection.publish(
        publication(kind, captures[kind], {
          ...(kind === "sa"
            ? { parent: f.connection.sender("s")!.producerId }
            : {}),
          deadlineEpochMs: originalDeadline,
          isCurrent: () => originalCurrent,
        }),
      );
    }
    const recv = f.device.transports[0];
    originalCurrent = false;
    vi.setSystemTime(originalDeadline + 1);
    const deadline = Date.now() + 10_000;
    const live = new Capture("live", "video");
    f.intercept((method, data) =>
      method === "produce" && (data as MediaRequests["produce"]).k === "l"
        ? Promise.reject(
            Object.assign(new Error("live_busy"), { code: "live_busy" }),
          )
        : undefined,
    );
    for (let attempt = 0; attempt < 8; attempt += 1) {
      await expect(
        f.connection.publish(
          publication("l", live, {
            lc: GENERATION,
            deadlineEpochMs: deadline,
            isCurrent: () => true,
          }),
        ),
      ).rejects.toMatchObject({ code: "live_busy" });
      expect(f.connection.sender("l")).toBeUndefined();
      expect(f.connection.sender("sa")!.epoch).toBe(EPOCH);
      expect(
        f.device.transports
          .flatMap((t) => t.producers)
          .filter((p) => p.track === live.native()),
      ).toEqual([]);
    }
    expect(recv.closed).toBe(false);
    expect(f.calls("closeTransport")).toHaveLength(0);
    expect(f.calls("closeTransport").length).toBeLessThanOrEqual(8);
    for (const transport of f.device.transports.slice(1)) {
      const parent = transport.producers.find(
        (p) => p.track === captures.s.native(),
      );
      const child = transport.produce.mock.calls.find(
        ([options]) =>
          (options.appData?.publication as MediaPublication).kind === "sa",
      );
      if (child)
        expect((child[0].appData?.publication as MediaPublication).parent).toBe(
          parent?.id,
        );
      for (const [options] of transport.produce.mock.calls)
        expect(options.stopTracks).toBe(false);
    }
    for (const [kind, capture] of Object.entries(captures)) {
      expect(
        f.connection.senders().find((s) => s.sourceKind === kind)?.track,
      ).toBe(capture.native());
      expect(capture.stop).not.toHaveBeenCalled();
    }
    expect(
      f.requests.filter(
        (r) =>
          r.method === "produce" &&
          (r.data as MediaRequests["produce"]).k === "l",
      ),
    ).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          deadlineEpochMs: deadline,
          data: expect.objectContaining({ epoch: EPOCH, lc: GENERATION }),
        }),
      ]),
    );
    expect(f.onError).not.toHaveBeenCalled();
    f.intercept(undefined);
    await f.connection.publish(
      publication("l", live, { lc: GENERATION, deadlineEpochMs: deadline }),
    );
    expect(f.connection.sender("l")!.track).toBe(live.native());
    expect(live.stop).not.toHaveBeenCalled();
  });

  it("passes one absolute deadline through fresh transport, native connect, and producer RPCs", async () => {
    const f = fixture();
    await f.start();
    f.device.configureSend = (transport) => {
      const original = transport.produce.getMockImplementation()!;
      transport.produce.mockImplementationOnce(async (options) => {
        await new Promise<void>((resolve, reject) =>
          transport.handlers.get("connect")?.(
            { dtlsParameters: { fingerprints: [] } },
            resolve,
            reject,
          ),
        );
        return original(options);
      });
    };
    const deadline = Date.now() + 10_000;
    await f.connection.publish(
      publication("l", new Capture("live", "video"), {
        deadlineEpochMs: deadline,
      }),
    );
    for (const method of ["transport", "connect", "produce"] as const) {
      const call = f.requests.findLast((r) => r.method === method);
      expect(call?.deadlineEpochMs).toBe(deadline);
      expect(call?.data).not.toHaveProperty("deadlineEpochMs");
    }
  });

  it("rejects an already expired publication before creating native resources", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("live", "video");
    await expect(
      f.connection.publish(
        publication("l", capture, {
          deadlineEpochMs: Date.now() - 1,
        }),
      ),
    ).rejects.toMatchObject({ code: "live_recovery_timeout" });
    expect(f.device.transports).toHaveLength(1);
    expect(f.calls("produce")).toEqual([]);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("uses the remaining absolute budget for a hung SDK send and retires its late producer", async () => {
    vi.useFakeTimers();
    const f = fixture();
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const send = f.device.transports[1],
      pending = deferred<NativeProducer>();
    send.produce.mockImplementationOnce(() => pending.promise);
    const capture = new Capture("live", "video");
    const publishing = f.connection.publish(
      publication("l", capture, { deadlineEpochMs: Date.now() + 25 }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "sdk_timeout",
    });
    await tick();
    await vi.advanceTimersByTimeAsync(26);
    await rejected;
    expect(send.closed).toBe(true);
    expect(f.calls("closeTransport")).toContainEqual({ transportId: send.id });
    const late = new NativeProducer("late-sdk-live", capture.native(), false);
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.connection.sender("l")).toBeUndefined();
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("closes a stale actual producer ID without retiring a newer publication of the same kind", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("live", "video");
    const old = await f.connection.publish(
      publication("l", capture, { lc: GENERATION }),
    );
    const pending = deferred<{ producerId: string }>();
    f.intercept((method) =>
      method === "produce" ? pending.promise : undefined,
    );
    let current = true;
    const publishing = f.connection.publish(
      publication("l", capture, {
        epoch: NEXT_EPOCH,
        lc: NEXT_EPOCH,
        isCurrent: () => current,
      }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "connection_closed",
    });
    await tick();
    current = false;
    pending.resolve({ producerId: "stale-server-live" });
    await rejected;
    expect(f.calls("closeProducer")).toContainEqual({
      producerId: "stale-server-live",
    });
    expect(f.connection.sender("l")).toBe(old);
    f.intercept(undefined);
    const next = await f.connection.publish(
      publication("l", capture, { epoch: NEXT_EPOCH, lc: NEXT_EPOCH }),
    );
    await f.connection.closeSource("l", old.producerId);
    await f.connection.closeSource("l", "stale-server-live");
    expect(f.connection.sender("l")).toBe(next);
    expect(f.calls("closeProducer")).not.toContainEqual({
      producerId: next.producerId,
    });
    await f.connection.closeSource("l", next.producerId);
    expect(f.connection.sender("l")).toBeUndefined();
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("retires an SDK producer resolving after publication ownership changes", async () => {
    const f = fixture();
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const pending = deferred<NativeProducer>();
    f.device.transports[1].produce.mockImplementationOnce(
      () => pending.promise,
    );
    const capture = new Capture("live", "video");
    let current = true;
    const publishing = f.connection.publish(
      publication("l", capture, { isCurrent: () => current }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "publication_cancelled",
    });
    await tick();
    current = false;
    const late = new NativeProducer(
      "late-owned-sdk-live",
      capture.native(),
      false,
    );
    pending.resolve(late);
    await rejected;
    expect(late.closed).toBe(true);
    expect(f.calls("closeProducer")).toContainEqual({ producerId: late.id });
    expect(f.connection.sender("l")).toBeUndefined();
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("closes a freshly created send transport if its reply exhausts the publication budget", async () => {
    vi.useFakeTimers();
    const f = fixture();
    await f.start();
    const pending = deferred<TransportOptions>();
    f.intercept((method, data) =>
      method === "transport" &&
      (data as MediaRequests["transport"]).direction === "send"
        ? pending.promise
        : undefined,
    );
    const capture = new Capture("live", "video"),
      deadline = Date.now() + 100;
    const publishing = f.connection.publish(
      publication("l", capture, { deadlineEpochMs: deadline }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "live_recovery_timeout",
    });
    await tick();
    vi.setSystemTime(deadline + 1);
    pending.resolve({
      id: "late-send",
      iceParameters: { usernameFragment: "u", password: "p" },
      iceCandidates: [],
      dtlsParameters: { fingerprints: [] },
    });
    await rejected;
    expect(f.device.transports.find((t) => t.id === "late-send")?.closed).toBe(
      true,
    );
    expect(f.calls("closeTransport")).toContainEqual({
      transportId: "late-send",
    });
    expect(f.calls("produce")).toEqual([]);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("does not reset the deadline when a compaction close reply exhausts the budget", async () => {
    vi.useFakeTimers();
    const f = fixture();
    f.device.handlerName = "Firefox120";
    await f.start();
    const capture = new Capture("screen", "video");
    for (let i = 0; i < 16; i += 1) {
      await f.connection.publish(publication("s", capture));
      await f.connection.closeSource("s");
    }
    const old = f.device.transports[1],
      pending = deferred<Record<string, never>>();
    f.intercept((method) =>
      method === "closeTransport" ? pending.promise : undefined,
    );
    const deadline = Date.now() + 100;
    const publishing = f.connection.publish(
      publication("l", capture, { deadlineEpochMs: deadline }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "live_recovery_timeout",
    });
    await tick();
    vi.setSystemTime(deadline + 1);
    pending.resolve({});
    await rejected;
    expect(old.closed).toBe(true);
    expect(
      // Failure cleanup has its own bounded RPC; the operation's first close
      // must retain the shared deadline rather than start another budget.
      f.requests.find((r) => r.method === "closeTransport")?.deadlineEpochMs,
    ).toBe(deadline);
    // The fake RPC accepts an expired budget, unlike MediaPeer.request. Any
    // native resource it still returns must be retired by failure cleanup.
    expect(f.device.transports.every((transport) => transport.closed)).toBe(
      true,
    );
    expect(f.calls("produce")).toHaveLength(16);
    expect(f.connection.sender("l")).toBeUndefined();
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("inherits the same remaining budget when compaction republishes current mic before video children", async () => {
    vi.useFakeTimers();
    const f = fixture();
    f.device.handlerName = "Firefox120";
    await f.start();
    const mic = new Capture("mic", "audio"),
      screen = new Capture("screen", "video"),
      audio = new Capture("screen-audio", "audio");
    await f.connection.publish(publication("a", mic));
    const parent = await f.connection.publish(publication("s", screen));
    await f.connection.publish(
      publication("sa", audio, { parent: parent.producerId }),
    );
    f.intercept((method) =>
      method === "produce"
        ? Promise.reject(
            Object.assign(new Error("live_busy"), { code: "live_busy" }),
          )
        : undefined,
    );
    const live = new Capture("live", "video");
    await expect(
      f.connection.publish(publication("l", live)),
    ).rejects.toMatchObject({ code: "live_busy" });
    f.intercept(undefined);
    for (let count = 0; count < 15; count++) {
      await f.connection.publish(
        publication("v", new Capture("camera-" + count, "video")),
      );
      await f.connection.closeSource("v");
    }
    const pending = deferred<NativeProducer>();
    f.device.configureSend = (transport) =>
      transport.produce.mockImplementationOnce(() => pending.promise);
    const publishing = f.connection.publish(
      publication("l", live, { deadlineEpochMs: Date.now() + 25 }),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "sdk_timeout",
    });
    await tick();
    const newSend = f.device.transports[2];
    expect(newSend.produce).toHaveBeenCalledOnce();
    expect(
      (
        newSend.produce.mock.calls[0][0].appData
          ?.publication as MediaPublication
      ).kind,
    ).toBe("a");
    await vi.advanceTimersByTimeAsync(26);
    await rejected;
    expect(newSend.closed).toBe(true);
    expect(f.calls("closeTransport")).toContainEqual({
      transportId: newSend.id,
    });
    expect(newSend.produce).toHaveBeenCalledOnce();
    const late = new NativeProducer("late-reproduced-mic", mic.native(), false);
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.connection.senders()).toEqual([]);
    for (const capture of [mic, screen, audio, live])
      expect(capture.stop).not.toHaveBeenCalled();
  });
});
afterEach(() => {
  for (const connection of active) connection.close();
  active.clear();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("public mediasoup connection lifecycle", () => {
  it("announces consumerReady only after actual SDK consume and local paused attachment", async () => {
    const f = fixture();
    await f.start();
    const recv = f.device.transports[0],
      pending = deferred<NativeConsumer>();
    recv.consumeGate = pending;
    f.connection.handleEvent(announcement());
    await tick();
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
    const consumer = new NativeConsumer("consumer-1");
    pending.resolve(consumer);
    await tick();
    expect(consumer.paused).toBe(true);
    expect(f.trace.indexOf("consume:consumer-1")).toBeLessThan(
      f.trace.indexOf("attach:consumer-1"),
    );
    expect(f.trace.indexOf("attach:consumer-1")).toBeLessThan(
      f.trace.indexOf("rpc:consumerReady"),
    );
    expect(f.calls("consumerReady")).toEqual([
      { consumerId: "consumer-1", generation: GENERATION },
    ]);
    f.connection.handleEvent({
      op: "consumerState",
      consumerId: "consumer-1",
      generation: GENERATION,
      paused: false,
    });
    expect(consumer.paused).toBe(false);
  });

  it("uses the server subscription generation and ignores stale state/closed tombstones", async () => {
    const f = fixture();
    await f.start();
    f.connection.handleEvent({
      op: "consumerClosed",
      consumerId: "consumer-1",
      generation: GENERATION,
    });
    f.connection.handleEvent({
      op: "consumerState",
      consumerId: "consumer-1",
      generation: GENERATION,
      paused: false,
    });
    f.connection.handleEvent(announcement({ generation: NEXT_EPOCH }));
    await tick();
    expect(f.attached).toHaveLength(1);
    expect(f.calls("consumerReady")).toEqual([
      { consumerId: "consumer-1", generation: NEXT_EPOCH },
    ]);
    const consumer = f.device.transports[0].consumers[0];
    expect(consumer.paused).toBe(true);
    f.connection.handleEvent({
      op: "consumerClosed",
      consumerId: "consumer-1",
      generation: GENERATION,
    });
    expect(consumer.closed).toBe(false);
    f.connection.handleEvent({
      op: "consumerState",
      consumerId: "consumer-1",
      generation: NEXT_EPOCH,
      paused: false,
    });
    expect(consumer.paused).toBe(false);
    f.connection.handleEvent({
      op: "consumerClosed",
      consumerId: "consumer-1",
      generation: NEXT_EPOCH,
    });
    expect(consumer.closed).toBe(true);
    expect(f.connection.consumers()).toEqual([]);
  });

  it("rejects removed or closed consumers before Ready/attachment", async () => {
    for (const closing of ["connection", "consumer"] as const) {
      const f = fixture();
      await f.start();
      const pending = deferred<NativeConsumer>();
      f.device.transports[0].consumeGate = pending;
      f.connection.handleEvent(announcement());
      await tick();
      if (closing === "connection") f.connection.close();
      else
        f.connection.handleEvent({
          op: "consumerClosed",
          consumerId: "consumer-1",
          generation: GENERATION,
        });
      const consumer = new NativeConsumer("consumer-1");
      pending.resolve(consumer);
      await tick();
      expect(consumer.closed).toBe(true);
      expect(f.attached).toEqual([]);
      expect(f.calls("consumerReady")).toEqual([]);
    }
  });

  it("does not send consumerReady if the attachment callback closes the connection", async () => {
    const f = fixture();
    await f.start();
    f.onConsumer.mockImplementation(() => f.connection.close());
    f.connection.handleEvent(announcement());
    await tick();
    expect(f.connection.consumers()).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
  });

  it("reports genuine consume failure and cleans the held receiver without Ready", async () => {
    const f = fixture();
    await f.start();
    f.device.transports[0].consume.mockRejectedValueOnce(
      new Error("native receive failed"),
    );
    f.connection.handleEvent(announcement());
    await tick();
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
    expect(f.calls("consumerFailed")).toEqual([
      { consumerId: "consumer-1", generation: GENERATION },
    ]);
    expect(f.onError).toHaveBeenCalledOnce();
  });

  it("closes and detaches a consumed receiver when consumerReady is rejected", async () => {
    const f = fixture();
    await f.start();
    f.intercept((method) =>
      method === "consumerReady"
        ? Promise.reject(new Error("subscription revoked"))
        : undefined,
    );
    f.connection.handleEvent(announcement());
    await tick();
    expect(f.device.transports[0].consumers[0].closed).toBe(true);
    expect(f.connection.consumers()).toEqual([]);
    expect(f.onConsumerClosed).toHaveBeenCalledOnce();
    expect(f.calls("consumerFailed")).toEqual([
      { consumerId: "consumer-1", generation: GENERATION },
    ]);
    expect(f.onError).toHaveBeenCalledOnce();
  });

  it("never constructs an SDK transport from a server response delivered after close", async () => {
    const f = fixture(),
      pending = deferred<TransportOptions>();
    f.intercept((method) =>
      method === "transport" ? pending.promise : undefined,
    );
    const starting = f.start(),
      rejected = expect(starting).rejects.toThrow("connection_closed");
    await tick();
    f.connection.close();
    pending.resolve({
      id: "late-transport",
      iceParameters: { usernameFragment: "u", password: "p" },
      iceCandidates: [],
      dtlsParameters: { fingerprints: [] },
    });
    await rejected;
    expect(f.device.transports).toEqual([]);
    expect(f.attached).toEqual([]);
  });

  it("does not acknowledge a native connect callback after its connection closes", async () => {
    const f = fixture();
    await f.start();
    const pending = deferred<Record<string, never>>();
    f.intercept((method) =>
      method === "connect" ? pending.promise : undefined,
    );
    const connected = vi.fn(),
      failed = vi.fn();
    f.device.transports[0].handlers.get("connect")?.(
      { dtlsParameters: { fingerprints: [] } },
      connected,
      failed,
    );
    expect(f.calls("connect")[0]).toMatchObject({
      transportId: f.device.transports[0].id,
    });
    f.connection.close();
    pending.resolve({});
    await tick();
    expect(connected).not.toHaveBeenCalled();
    expect(failed).toHaveBeenCalledOnce();
    expect(failed.mock.calls[0][0]).toMatchObject({
      code: "connection_closed",
    });
  });

  it("compacts receive transports and accepts only fresh server consumer announcements", async () => {
    const f = fixture();
    f.device.handlerName = "Firefox120";
    await f.start();
    const old = f.device.transports[0];
    for (let count = 0; count < 16; count += 1) {
      const consumerId = "consumer-" + count;
      f.connection.handleEvent(announcement({ consumerId }));
      await tick();
      f.connection.handleEvent({
        op: "consumerClosed",
        consumerId,
        generation: GENERATION,
      });
    }
    f.connection.handleEvent(announcement({ consumerId: "obsolete-fifth" }));
    await tick();
    expect(old.closed).toBe(true);
    expect(f.device.transports).toHaveLength(2);
    expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
    expect(f.trace.indexOf("transport-close:" + old.id)).toBeLessThan(
      f.trace.indexOf("rpc:closeTransport"),
    );
    expect(
      f
        .calls("consumerReady")
        .some((call) => call.consumerId === "obsolete-fifth"),
    ).toBe(false);
    expect(f.connection.consumers()).toEqual([]);
    f.connection.handleEvent(
      announcement({ consumerId: "fresh-server-consumer" }),
    );
    await tick();
    expect(f.device.transports[1].consume).toHaveBeenCalledOnce();
    expect(f.connection.consumers()[0].consumerId).toBe(
      "fresh-server-consumer",
    );
    expect(f.calls("consumerReady").at(-1)).toEqual({
      consumerId: "fresh-server-consumer",
      generation: GENERATION,
    });
    f.connection.handleEvent({
      op: "consumerState",
      consumerId: "fresh-server-consumer",
      generation: NEXT_EPOCH,
      paused: false,
    });
    expect(f.device.transports[1].consumers[0].paused).toBe(true);
  });

  it("keeps same-capture ownership with stopTracks:false through close and replacement", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("mic", "audio");
    const original = await f.connection.publish(publication("a", capture));
    const originalId = original.producerId;
    f.setCodec({ opusDtx: true, opusStereo: false });
    const changed = await f.connection.publish(publication("a", capture));
    expect(changed.producerId).not.toBe(originalId);
    const send = f.device.transports[1];
    for (const [options] of send.produce.mock.calls)
      expect(options).toMatchObject({
        track: capture.native(),
        stopTracks: false,
        disableTrackOnPause: false,
      });
    expect(send.producers[0].closed).toBe(true);
    await f.connection.closeSource("a");
    expect(capture.stop).not.toHaveBeenCalled();
    expect(capture.readyState).toBe("live");
    expect(f.connection.senders()).toEqual([]);
  });

  it("replaces Opus codec options transactionally with expected producer and same source epoch", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("mic", "audio");
    const original = await f.connection.publish(publication("a", capture));
    const originalId = original.producerId;
    f.setCodec({ opusDtx: true, opusStereo: false });
    const changed = await f.connection.publish(publication("a", capture));
    expect(f.calls("produce")).toMatchObject([
      { k: "a", epoch: EPOCH },
      { k: "a", epoch: EPOCH, expectedOldProducerId: originalId },
    ]);
    expect(changed.epoch).toBe(EPOCH);
    f.connection.handleEvent({
      op: "producerClosed",
      producerId: originalId,
      epoch: EPOCH,
    });
    expect(f.connection.sender("a")).toBe(changed);
    f.connection.handleEvent({
      op: "producerClosed",
      producerId: changed.producerId,
      epoch: NEXT_EPOCH,
    });
    expect(f.connection.sender("a")).toBe(changed);
  });

  it("retains the previous producer when replacement RPC fails", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("mic", "audio");
    const original = await f.connection.publish(publication("a", capture));
    f.setCodec({ opusDtx: true });
    f.intercept((method) =>
      method === "produce"
        ? Promise.reject(new Error("replacement denied"))
        : undefined,
    );
    await expect(
      f.connection.publish(publication("a", capture)),
    ).rejects.toThrow("replacement denied");
    expect(f.connection.sender("a")).toBe(original);
    expect(f.device.transports[1].producers[0].closed).toBe(false);
    expect(f.device.transports[1].closed).toBe(false);
    expect(f.device.transports).toHaveLength(2);
    expect(f.calls("closeTransport")).toEqual([]);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("rebuilds one tainted native send transport preserving current captures, epochs, privacy and audio parent IDs", async () => {
    const f = fixture();
    await f.start();
    f.connection.handleEvent(announcement());
    await tick();
    const recv = f.device.transports[0],
      receiver = f.connection.consumers()[0];
    const mic = new Capture("mic", "audio"),
      camera = new Capture("camera", "video"),
      screen = new Capture("screen-initial", "video"),
      currentScreen = new Capture("screen-current", "video"),
      screenAudio = new Capture("screen-audio", "audio"),
      live = new Capture("live", "video");
    const micSender = await f.connection.publish(
      publication("a", mic, { paused: true }),
    );
    await f.connection.publish(publication("v", camera, { epoch: NEXT_EPOCH }));
    const screenSender = await f.connection.publish(publication("s", screen));
    await f.connection.publish(
      publication("sa", screenAudio, { parent: screenSender.producerId }),
    );
    await screenSender.replaceTrack(currentScreen.native());
    const old = f.device.transports[1],
      oldMicId = micSender.producerId,
      oldScreenId = screenSender.producerId;
    old.failNativeSendOnce = true;
    await expect(
      f.connection.publish(publication("l", live, { epoch: NEXT_EPOCH })),
    ).rejects.toMatchObject({ name: "OperationError" });
    expect(old.nativeSendTainted).toBe(true);
    expect(old.closed).toBe(true);
    expect(f.device.transports).toHaveLength(3);
    expect(f.calls("transport")).toEqual([
      { direction: "recv" },
      { direction: "send" },
      { direction: "send" },
    ]);
    expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
    expect(f.trace.indexOf("transport-close:" + old.id)).toBeLessThan(
      f.trace.indexOf("rpc:closeTransport"),
    );
    const fresh = f.device.transports[2],
      inputs = fresh.produce.mock.calls.map(([options]) => options);
    expect(inputs.map((input) => input.track)).toEqual([
      mic.native(),
      camera.native(),
      currentScreen.native(),
      screenAudio.native(),
    ]);
    expect(inputs.map((input) => input.appData?.publication)).toMatchObject([
      { kind: "a", epoch: EPOCH, paused: true },
      { kind: "v", epoch: NEXT_EPOCH },
      { kind: "s", epoch: EPOCH },
      {
        kind: "sa",
        epoch: EPOCH,
        parent: f.connection.sender("s")!.producerId,
      },
    ]);
    for (const input of inputs)
      expect(input).toMatchObject({
        stopTracks: false,
        disableTrackOnPause: false,
      });
    expect(fresh.producers[0].paused).toBe(true);
    expect(f.connection.sender("a")!.producerId).not.toBe(oldMicId);
    expect(f.connection.sender("s")!.producerId).not.toBe(oldScreenId);
    expect(recv.closed).toBe(false);
    expect(recv.close).not.toHaveBeenCalled();
    expect(f.connection.consumers()).toEqual([receiver]);
    expect(recv.consumers[0].closed).toBe(false);
    f.connection.handleEvent({
      op: "producerClosed",
      producerId: oldMicId,
      epoch: EPOCH,
    });
    f.connection.handleEvent({
      op: "producerClosed",
      producerId: f.connection.sender("a")!.producerId,
      epoch: NEXT_EPOCH,
    });
    expect(f.connection.sender("a")).toBeDefined();
    const retry = await f.connection.publish(
      publication("l", live, { epoch: NEXT_EPOCH }),
    );
    expect(retry.track).toBe(live.native());
    expect(retry.epoch).toBe(NEXT_EPOCH);
    expect(f.device.transports).toHaveLength(3);
    expect(fresh.produce).toHaveBeenCalledTimes(5);
    for (const capture of [
      mic,
      camera,
      screen,
      currentScreen,
      screenAudio,
      live,
    ]) {
      expect(capture.stop).not.toHaveBeenCalled();
      expect(capture.readyState).toBe("live");
    }
  });

  it("reports a failed send rebuild after one fresh transport without recursive native OperationError retries", async () => {
    const f = fixture();
    await f.start();
    const mic = new Capture("mic", "audio");
    await f.connection.publish(publication("a", mic));
    const old = f.device.transports[1];
    old.failNativeSendOnce = true;
    f.device.configureSend = (transport) => {
      transport.failNativeSendOnce = true;
    };
    await expect(
      f.connection.publish(publication("l", new Capture("live", "video"))),
    ).rejects.toMatchObject({ name: "OperationError" });
    expect(old.closed).toBe(true);
    expect(f.device.transports).toHaveLength(3);
    expect(f.device.transports[2].produce).toHaveBeenCalledOnce();
    expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
    expect(f.onTransportState).toHaveBeenCalledWith("send", "failed");
    expect(mic.stop).not.toHaveBeenCalled();
  });

  it("retires a tainted native transport when single-layer SDK fallback rejects its SDP", async () => {
    const f = fixture();
    await f.start();
    const mic = new Capture("mic", "audio"),
      live = new Capture("live", "video");
    await f.connection.publish(publication("a", mic));
    const old = f.device.transports[1];
    old.produce.mockRejectedValueOnce(
      Object.assign(new Error("simulcast unavailable"), {
        name: "UnsupportedError",
      }),
    );
    old.failNativeSendOnce = true;
    await expect(
      f.connection.publish(publication("l", live)),
    ).rejects.toMatchObject({
      name: "OperationError",
    });
    expect(old.produce).toHaveBeenCalledTimes(3);
    expect(old.produce.mock.calls[1][0].encodings).toHaveLength(2);
    expect(old.produce.mock.calls[2][0]).not.toHaveProperty("encodings");
    expect(old.nativeSendTainted).toBe(true);
    expect(old.closed).toBe(true);
    expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
    expect(f.device.transports).toHaveLength(3);
    const retry = await f.connection.publish(publication("l", live));
    expect(retry.track).toBe(live.native());
    expect(f.device.transports).toHaveLength(3);
    expect(mic.stop).not.toHaveBeenCalled();
    expect(live.stop).not.toHaveBeenCalled();
  });

  for (const realm of ["native DOMException", "foreign realm Error"] as const)
    it(`rebuilds a tainted public transport after a genuine ${realm} OperationError`, async () => {
      const f = fixture();
      await f.start();
      const mic = new Capture("mic", "audio"),
        live = new Capture("live", "video");
      await f.connection.publish(publication("a", mic));
      const old = f.device.transports[1];
      const failure =
        realm === "native DOMException"
          ? new DOMException("setRemoteDescription rejected", "OperationError")
          : await foreignNativeOperationError();
      if (realm === "foreign realm Error")
        expect(failure instanceof Error).toBe(false);
      old.nativeSendTainted = true;
      old.produce.mockRejectedValueOnce(failure);
      await expect(
        f.connection.publish(publication("l", live)),
      ).rejects.toMatchObject({
        name: "OperationError",
      });
      expect(old.closed).toBe(true);
      expect(f.device.transports).toHaveLength(3);
      expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
      const retry = await f.connection.publish(publication("l", live));
      expect(retry.track).toBe(live.native());
      expect(f.device.transports).toHaveLength(3);
      expect(f.device.transports[0].closed).toBe(false);
      expect(mic.stop).not.toHaveBeenCalled();
      expect(live.stop).not.toHaveBeenCalled();
    });

  it("bounds a hung SDK producer during native send rebuilding", async () => {
    vi.useFakeTimers();
    const f = fixture("voice", 20);
    await f.start();
    const mic = new Capture("mic", "audio");
    await f.connection.publish(publication("a", mic));
    f.device.transports[1].failNativeSendOnce = true;
    f.device.configureSend = (transport) => {
      transport.produce.mockImplementationOnce(() => new Promise(() => {}));
    };
    const publishing = f.connection.publish(
      publication("l", new Capture("live", "video")),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "sdk_timeout",
    });
    for (let count = 0; count < 5; count += 1) await tick();
    expect(f.device.transports).toHaveLength(3);
    expect(f.device.transports[2].produce).toHaveBeenCalledOnce();
    await vi.advanceTimersByTimeAsync(21);
    await rejected;
    expect(f.onTransportState).toHaveBeenCalledWith("send", "failed");
    expect(f.device.transports.every((transport) => transport.closed)).toBe(
      true,
    );
    expect(f.connection.senders()).toEqual([]);
    expect(mic.stop).not.toHaveBeenCalled();
  });

  it("retires a late rebuilt SDK producer after close without reviving a stale connection", async () => {
    const f = fixture();
    await f.start();
    const mic = new Capture("mic", "audio"),
      pending = deferred<NativeProducer>();
    await f.connection.publish(publication("a", mic));
    f.device.transports[1].failNativeSendOnce = true;
    f.device.configureSend = (transport) => {
      transport.produce.mockImplementationOnce(() => pending.promise);
    };
    const publishing = f.connection.publish(
      publication("l", new Capture("live", "video")),
    );
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "connection_closed",
    });
    for (let count = 0; count < 5; count += 1) await tick();
    expect(f.device.transports).toHaveLength(3);
    expect(f.device.transports[2].produce).toHaveBeenCalledOnce();
    f.connection.close();
    await rejected;
    const late = new NativeProducer(
      "late-rebuilt-producer",
      mic.native(),
      false,
    );
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.connection.senders()).toEqual([]);
    expect(f.calls("transport")).toHaveLength(3);
    expect(mic.stop).not.toHaveBeenCalled();
  });

  it("sets only a video startup hint for VP8 scale 4/1 and its single-layer fallback, preserving Opus options", async () => {
    const f = fixture();
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const send = f.device.transports[1];
    expect(send.produce.mock.calls[0][0].codecOptions).toEqual({
      opusDtx: false,
      opusStereo: true,
    });
    f.setCodec({});
    await f.connection.publish(
      publication("v", new Capture("camera", "video")),
    );
    send.productionErrors.push(
      Object.assign(new Error("simulcast unavailable"), {
        name: "UnsupportedError",
      }),
    );
    await f.connection.publish(
      publication("s", new Capture("screen", "video")),
    );
    const videoAttempts = send.produce.mock.calls
      .slice(1)
      .map(([options]) => options);
    expect(videoAttempts).toHaveLength(3);
    expect(videoAttempts[0].encodings).toEqual([
      { scaleResolutionDownBy: 4 },
      { scaleResolutionDownBy: 1 },
    ]);
    expect(videoAttempts[1].encodings).toEqual(videoAttempts[0].encodings);
    expect(videoAttempts[2]).not.toHaveProperty("encodings");
    for (const attempt of videoAttempts) {
      expect(attempt.codecOptions).toEqual({ videoGoogleStartBitrate: 1000 });
      for (const encoding of attempt.encodings ?? []) {
        expect(encoding).not.toHaveProperty("maxBitrate");
        expect(encoding).not.toHaveProperty("minBitrate");
      }
      expect(attempt.stopTracks).toBe(false);
    }
  });

  it("uses one same-capture single-layer fallback only for supported SDK error classes", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("video", "video");
    // Create the send transport through another independently held source.
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const send = f.device.transports[1];
    send.productionErrors.push(
      Object.assign(new Error("simulcast unavailable"), {
        name: "UnsupportedError",
      }),
    );
    await f.connection.publish(publication("s", capture));
    const attempts = send.produce.mock.calls
      .slice(-2)
      .map(([options]) => options);
    expect(attempts[0].encodings).toHaveLength(2);
    expect(attempts[1]).not.toHaveProperty("encodings");
    for (const attempt of attempts)
      expect(attempt).toMatchObject({
        track: capture.native(),
        stopTracks: false,
      });
    expect(f.calls("produce")).toHaveLength(2);
    expect(capture.stop).not.toHaveBeenCalled();
    send.productionErrors.push(new Error("network disconnected"));
    await expect(
      f.connection.publish(publication("l", capture)),
    ).rejects.toThrow("network disconnected");
    expect(f.connection.sender("l")).toBeUndefined();
    expect(f.calls("produce")).toHaveLength(2);
  });

  it("closes a producer acknowledged after connection close instead of publishing it", async () => {
    const f = fixture();
    await f.start();
    const pending = deferred<{ producerId: string }>();
    f.intercept((method) =>
      method === "produce" ? pending.promise : undefined,
    );
    const capture = new Capture("mic", "audio"),
      publishing = f.connection.publish(publication("a", capture));
    const rejected = expect(publishing).rejects.toThrow("connection_closed");
    await tick();
    f.connection.close();
    pending.resolve({ producerId: "late-producer" });
    await rejected;
    expect(f.calls("closeProducer")).toEqual([{ producerId: "late-producer" }]);
    expect(f.connection.sender("a")).toBeUndefined();
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("pauses native publication and sends exact source ownership without stopping capture", async () => {
    const f = fixture();
    await f.start();
    const capture = new Capture("mic", "audio");
    const sender = await f.connection.publish(
      publication("a", capture, { paused: true }),
    );
    const producer = f.device.transports[1].producers[0];
    expect(f.calls("produce")[0]).toMatchObject({ k: "a", paused: true });
    expect(producer.paused).toBe(true);
    expect(f.calls("pauseProducer")).toEqual([]);
    await f.connection.setSourcePaused("a", false);
    await f.connection.setSourcePaused("a", true);
    expect(f.calls("resumeProducer")).toEqual([
      { producerId: sender.producerId },
    ]);
    expect(f.calls("pauseProducer")).toEqual([
      { producerId: sender.producerId },
    ]);
    expect(producer.paused).toBe(true);
    expect(capture.enabled).toBe(true);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("compacts through public transports with current capture and fresh child parent IDs", async () => {
    const f = fixture();
    f.device.handlerName = "Firefox120";
    await f.start();
    const screen = new Capture("screen-old", "video"),
      current = new Capture("screen-current", "video");
    const video = await f.connection.publish(publication("s", screen));
    const audio = new Capture("screen-audio", "audio");
    await f.connection.publish(
      publication("sa", audio, { parent: video.producerId }),
    );
    await video.replaceTrack(current.native());
    for (let count = 0; count < 16; count += 1) {
      await f.connection.publish(
        publication("v", new Capture("camera-" + count, "video")),
      );
      await f.connection.closeSource("v");
    }
    const old = f.device.transports[1];
    await f.connection.publish(
      publication("v", new Capture("camera-final", "video")),
    );
    expect(old.closed).toBe(true);
    expect(f.device.transports).toHaveLength(3);
    expect(f.calls("closeTransport")).toEqual([{ transportId: old.id }]);
    expect(f.trace.indexOf("transport-close:" + old.id)).toBeLessThan(
      f.trace.indexOf("rpc:closeTransport"),
    );
    const fresh = f.device.transports[2],
      newVideo = f.connection.sender("s")!;
    const republished = fresh.produce.mock.calls.map(([options]) => options);
    expect(republished[0].track).toBe(current.native());
    expect(republished[1].track).toBe(audio.native());
    const produced = f.calls("produce").slice(-3);
    expect(produced[0]).toMatchObject({ k: "s", epoch: EPOCH });
    expect(produced[1]).toMatchObject({
      k: "sa",
      epoch: EPOCH,
      parent: newVideo.producerId,
    });
    expect(produced[1].parent).not.toBe(video.producerId);
    expect(screen.stop).not.toHaveBeenCalled();
    expect(current.stop).not.toHaveBeenCalled();
    expect(audio.stop).not.toHaveBeenCalled();
  });

  it("passes fresh native sender parameters so clearing maxBitrate reaches the SDK sender", async () => {
    const f = fixture();
    await f.start();
    const sender = await f.connection.publish(
      publication("a", new Capture("mic", "audio")),
    );
    const native = f.device.transports[1].producers[0].rtpSender;
    expect(sender.rtpSender).toBe(native);
    const parameters = sender.getParameters();
    delete parameters.encodings[0].maxBitrate;
    await sender.setParameters(parameters);
    expect(native.getParameters()).toEqual({
      transactionId: "native-transaction",
      encodings: [{ maxFramerate: 30 }],
    });
    expect(native.setParameters).toHaveBeenCalledOnce();
  });

  it("never revives a detached microphone capture during send transport compaction", async () => {
    const f = fixture();
    f.device.handlerName = "Firefox120";
    await f.start();
    const capture = new Capture("detached-mic", "audio");
    const mic = await f.connection.publish(publication("a", capture));
    await mic.replaceTrack(null);
    for (let count = 0; count < 16; count += 1) {
      await f.connection.publish(
        publication("v", new Capture("camera-" + count, "video")),
      );
      await f.connection.closeSource("v");
    }
    // An explicit unavailable-capture error is acceptable; reviving the old
    // microphone to complete a transport replacement is not.
    await f.connection
      .publish(publication("v", new Capture("camera-final", "video")))
      .catch(() => {});
    expect(f.device.transports[1].closed).toBe(true);
    const republished = f.device.transports
      .slice(2)
      .flatMap((transport) =>
        transport.produce.mock.calls.map(([options]) => options.track),
      );
    expect(republished).not.toContain(capture.native());
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("keeps separate connections' RPC ownership and refuses watch publication", async () => {
    const voice = fixture(),
      watch = fixture("watch");
    await voice.start();
    await watch.start();
    const capture = new Capture("mic", "audio"),
      sender = await voice.connection.publish(publication("a", capture));
    await voice.connection.closeSource("a");
    expect(voice.calls("closeProducer")).toEqual([
      { producerId: sender.producerId },
    ]);
    expect(watch.calls("closeProducer")).toEqual([]);
    await expect(
      watch.connection.publish(publication("a", capture)),
    ).rejects.toThrow("watch_read_only");
    expect(watch.calls("transport")).toEqual([{ direction: "recv" }]);
    expect(watch.calls("produce")).toEqual([]);
  });

  it("bounds a never-resolving SDK consume and retires a consumer delivered after timeout", async () => {
    vi.useFakeTimers();
    const f = fixture("voice", 20);
    await f.start();
    const pending = deferred<NativeConsumer>();
    f.device.transports[0].consumeGate = pending;
    f.connection.handleEvent(announcement());
    await tick();
    await vi.advanceTimersByTimeAsync(21);
    expect(f.onError).toHaveBeenCalledWith(
      expect.objectContaining({ code: "sdk_timeout" }),
    );
    expect(f.device.transports[0].closed).toBe(true);
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
    const late = new NativeConsumer("consumer-1");
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.connection.consumers()).toEqual([]);
  });

  it("cancels a hung SDK consume immediately on close and retires its late receiver", async () => {
    const f = fixture();
    await f.start();
    const pending = deferred<NativeConsumer>();
    f.device.transports[0].consumeGate = pending;
    f.connection.handleEvent(announcement());
    await tick();
    f.connection.close();
    await tick();
    expect(f.onError).not.toHaveBeenCalled();
    const late = new NativeConsumer("consumer-1");
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
  });

  it("bounds never-resolving SDK production and closes a late producer without stopping capture", async () => {
    vi.useFakeTimers();
    const f = fixture("voice", 20);
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const capture = new Capture("screen", "video"),
      pending = deferred<NativeProducer>();
    f.device.transports[1].produce.mockImplementationOnce(
      () => pending.promise,
    );
    const publishing = f.connection.publish(publication("s", capture));
    const rejected = expect(publishing).rejects.toMatchObject({
      code: "sdk_timeout",
    });
    await tick();
    await vi.advanceTimersByTimeAsync(21);
    await rejected;
    const late = new NativeProducer(
      "late-native-producer",
      capture.native(),
      false,
    );
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
    expect(f.connection.senders()).toEqual([]);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("rejects a late SDK resolution by elapsed time even before its timer executes", async () => {
    const f = fixture("voice", 20);
    await f.start();
    let elapsed = 0;
    vi.spyOn(performance, "now").mockImplementation(() => elapsed);
    const consumer = new NativeConsumer("consumer-1");
    f.device.transports[0].consume.mockImplementationOnce(async () => {
      elapsed = 21;
      return consumer;
    });
    f.connection.handleEvent(announcement());
    await tick();
    expect(consumer.closed).toBe(true);
    expect(f.onError).toHaveBeenCalledWith(
      expect.objectContaining({ code: "sdk_timeout" }),
    );
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
  });

  it("keeps a stats timeout unavailable without closing a working connection", async () => {
    vi.useFakeTimers();
    const f = fixture("voice", 20);
    await f.start();
    f.device.transports[0].getStats.mockImplementationOnce(
      () => new Promise(() => {}),
    );
    const stats = f.connection.getStats(),
      rejected = expect(stats).rejects.toMatchObject({ code: "sdk_timeout" });
    await tick();
    await vi.advanceTimersByTimeAsync(21);
    await rejected;
    expect(f.device.transports[0].closed).toBe(false);
    expect(f.onError).not.toHaveBeenCalled();
    await expect(f.connection.getStats()).resolves.toEqual([]);
  });

  it("collects public producer/consumer stats and deduplicates rows per transport without summing counters", async () => {
    const f = fixture();
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    f.connection.handleEvent(announcement());
    await tick();
    const [recv, send] = f.device.transports;
    const outbound = {
      id: "rtp-row",
      type: "outbound-rtp",
      timestamp: 100,
      kind: "audio",
      ssrc: 17,
      packetsSent: 50,
      bytesSent: 500,
      codecId: "opus-codec",
    };
    const inbound = {
      id: "rtp-row",
      type: "inbound-rtp",
      timestamp: 100,
      kind: "audio",
      ssrc: 23,
      packetsReceived: 40,
      bytesReceived: 400,
    };
    send.getStats.mockResolvedValue(new Map([[outbound.id, outbound]]));
    send.producers[0].getStats.mockResolvedValue(
      new Map([[outbound.id, outbound]]),
    );
    recv.getStats.mockResolvedValue(new Map([[inbound.id, inbound]]));
    recv.consumers[0].getStats.mockResolvedValue(
      new Map([[inbound.id, inbound]]),
    );
    const rows = await f.connection.getStats();
    expect(rows).toHaveLength(2);
    expect(rows).toContainEqual(
      expect.objectContaining({
        id: `${send.id}:rtp-row`,
        packetsSent: 50,
        bytesSent: 500,
        codecId: `${send.id}:opus-codec`,
      }),
    );
    expect(rows).toContainEqual(
      expect.objectContaining({
        id: `${recv.id}:rtp-row`,
        packetsReceived: 40,
        bytesReceived: 400,
      }),
    );
    expect(send.producers[0].getStats).toHaveBeenCalledOnce();
    expect(recv.consumers[0].getStats).toHaveBeenCalledOnce();
  });

  it("rejects send queue overflow and settles every queued publication on close", async () => {
    const f = fixture();
    await f.start();
    await f.connection.publish(publication("a", new Capture("mic", "audio")));
    const pending = deferred<NativeProducer>();
    f.device.transports[1].produce.mockImplementationOnce(
      () => pending.promise,
    );
    const capture = new Capture("screen", "video");
    const queued = Array.from({ length: 64 }, () =>
      f.connection.publish(publication("s", capture)),
    );
    const settled = Promise.allSettled(queued);
    await tick();
    await expect(
      f.connection.publish(publication("l", capture)),
    ).rejects.toMatchObject({ code: "request_overflow" });
    f.connection.close();
    const results = await settled;
    expect(results.every((result) => result.status === "rejected")).toBe(true);
    expect(capture.stop).not.toHaveBeenCalled();
  });

  it("accepts more than 64 announced consumers without interrupting existing media", async () => {
    const f = fixture();
    await f.start();
    for (let count = 0; count < 100; count++) {
      f.connection.handleEvent(announcement({ consumerId: "many-" + count }));
    }
    await vi.waitFor(() => expect(f.connection.consumers()).toHaveLength(100));
    expect(f.calls("consumerReady")).toHaveLength(100);
    expect(f.onError).not.toHaveBeenCalled();
    expect(f.device.transports).toHaveLength(1);
    expect(f.device.transports[0].closed).toBe(false);
  });

  it.each(["Chrome111", "Firefox120"])(
    "keeps mic and receive transport stable for ordinary camera toggles in %s",
    async (handler) => {
      const f = fixture();
      f.device.handlerName = handler;
      await f.start();
      const mic = await f.connection.publish(
        publication("a", new Capture("mic", "audio")),
      );
      const recv = f.device.transports[0];
      for (let count = 0; count < 10; count++) {
        await f.connection.publish(
          publication("v", new Capture("camera", "video")),
        );
        await f.connection.closeSource("v");
        f.connection.handleEvent(
          announcement({ consumerId: "camera-" + count }),
        );
        await tick();
        f.connection.handleEvent({
          op: "consumerClosed",
          consumerId: "camera-" + count,
          generation: GENERATION,
        });
      }
      expect(f.connection.sender("a")?.producerId).toBe(mic.producerId);
      expect(f.calls("closeTransport")).toEqual([]);
      expect(recv.closed).toBe(false);
    },
  );

  it("fails closed on receive backlog overflow with no fabricated Ready or attachments", async () => {
    const f = fixture();
    await f.start();
    const pending = deferred<NativeConsumer>();
    f.device.transports[0].consumeGate = pending;
    f.connection.handleEvent(announcement({ consumerId: "consumer-0" }));
    await tick();
    for (let count = 1; count <= 1024; count += 1)
      f.connection.handleEvent(
        announcement({ consumerId: "consumer-" + count }),
      );
    await tick();
    expect(f.onError).toHaveBeenCalledWith(
      expect.objectContaining({ code: "event_overflow" }),
    );
    expect(f.device.transports[0].closed).toBe(true);
    expect(f.attached).toEqual([]);
    expect(f.calls("consumerReady")).toEqual([]);
    const late = new NativeConsumer("consumer-0");
    pending.resolve(late);
    await tick();
    expect(late.closed).toBe(true);
  });

  it("fails closed on pre-start event overflow before loading any public SDK transport", async () => {
    const f = fixture();
    for (let count = 0; count <= 3 * 1024; count += 1)
      f.connection.handleEvent(
        announcement({ consumerId: "consumer-" + count }),
      );
    await expect(f.start()).rejects.toMatchObject({
      code: "connection_closed",
    });
    expect(f.onError).toHaveBeenCalledWith(
      expect.objectContaining({ code: "event_overflow" }),
    );
    expect(f.device.load).not.toHaveBeenCalled();
    expect(f.device.transports).toEqual([]);
  });
});
