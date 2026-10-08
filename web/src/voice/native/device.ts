// mediasoup-client's Device/Transport/Producer/Consumer surface, backed by the
// desktop app's native core. MediasoupConnection drives it exactly like the
// SDK, so signaling, tickets, ACL, epochs and the consumer handshake stay in
// one place; only the media objects differ.
import type {
  Device,
  RtpCapabilities,
  RtpCodecCapability,
  RtpParameters,
} from "mediasoup-client/types";
import { invokeNative, nativeBridge } from "./bridge.ts";
import { NativeTrack, isNativeTrack } from "./tracks.ts";

type Listener = (...args: never[]) => void;

class Emitter {
  private listeners = new Map<string, Set<Listener>>();
  on(event: string, listener: Listener): this {
    let set = this.listeners.get(event);
    if (!set) this.listeners.set(event, (set = new Set()));
    set.add(listener);
    return this;
  }
  off(event: string, listener: Listener): this {
    this.listeners.get(event)?.delete(listener);
    return this;
  }
  protected emit(event: string, ...args: unknown[]): boolean {
    const set = this.listeners.get(event);
    if (!set?.size) return false;
    for (const listener of [...set])
      (listener as (...a: unknown[]) => void)(...args);
    return true;
  }
}

function report(error: unknown): void {
  console.warn("[gelabber] native media", error);
}

function closedError(): Error {
  return new DOMException("native transport closed", "InvalidStateError");
}

type TransportMessage =
  | { type: "connect"; request: number; dtlsParameters: unknown }
  | {
      type: "produce";
      request: number;
      kind: string;
      rtpParameters: RtpParameters;
      appData: unknown;
    }
  | { type: "connectionstatechange"; state: RTCPeerConnectionState };

type NativeTransportOptions = {
  id: string;
  iceParameters: unknown;
  iceCandidates: unknown[];
  dtlsParameters: unknown;
  iceServers?: unknown[];
  iceTransportPolicy?: string;
};

type SenderParameters = {
  encodings: Array<Record<string, unknown>>;
  transactionId?: string;
};

export class NativeProducer {
  closed = false;
  paused = false;
  readonly kind: "audio" | "video";
  readonly appData: Record<string, unknown>;
  private parameters: SenderParameters = { encodings: [] };
  private current: NativeTrack | null;
  readonly rtpSender: RTCRtpSender;

  constructor(
    readonly handle: number,
    readonly id: string,
    track: NativeTrack,
    readonly rtpParameters: RtpParameters,
    appData: Record<string, unknown> | undefined,
  ) {
    this.kind = track.kind;
    this.current = track;
    this.appData = appData ?? {};
    // RTCRtpSender.getParameters() is synchronous; keep the last answer.
    const sender = {
      getParameters: () => structuredClone(this.parameters),
      setParameters: async (next: SenderParameters) => {
        await invokeNative("media_producer_set_parameters", {
          producer: this.handle,
          parameters: { encodings: next.encodings },
        });
        await this.refreshParameters();
      },
    };
    this.rtpSender = sender as unknown as RTCRtpSender;
  }
  get track(): MediaStreamTrack | null {
    return this.current as unknown as MediaStreamTrack | null;
  }
  async refreshParameters(): Promise<void> {
    this.parameters = await invokeNative<SenderParameters>(
      "media_producer_parameters",
      { producer: this.handle },
    );
  }
  private setNativePaused(paused: boolean): void {
    if (this.closed) return;
    void invokeNative("media_producer_pause", {
      producer: this.handle,
      paused,
    }).catch(report);
  }
  pause(): void {
    this.paused = true;
    this.setNativePaused(true);
  }
  resume(): void {
    this.paused = false;
    if (this.current) this.setNativePaused(false);
  }
  /** `null` stops sending until a track is set again (paused natively). */
  async replaceTrack({
    track,
  }: {
    track: MediaStreamTrack | null;
  }): Promise<void> {
    if (this.closed) throw closedError();
    if (track === null) {
      this.current = null;
      this.setNativePaused(true);
      return;
    }
    if (!isNativeTrack(track) || track.handle.source === undefined)
      throw new TypeError("native producers send native capture tracks only");
    await invokeNative("media_producer_replace_source", {
      producer: this.handle,
      source: track.handle.source,
    });
    this.current = track;
    if (!this.paused) this.setNativePaused(false);
  }
  async getStats(): Promise<unknown[]> {
    return invokeNative<unknown[]>("media_producer_stats", {
      producer: this.handle,
    });
  }
  close(): void {
    if (this.closed) return;
    this.closed = true;
    void invokeNative("media_producer_close", { producer: this.handle }).catch(
      report,
    );
  }
}

export class NativeConsumer {
  closed = false;
  paused = false;
  readonly rtpReceiver: RTCRtpReceiver | undefined = undefined;
  private readonly remote: NativeTrack;

  constructor(
    readonly handle: number,
    readonly id: string,
    readonly producerId: string,
    readonly kind: "audio" | "video",
    readonly rtpParameters: RtpParameters,
    readonly appData: Record<string, unknown>,
  ) {
    this.remote = new NativeTrack(kind, `native ${kind} ${id}`, {
      consumer: handle,
    });
  }
  get track(): MediaStreamTrack {
    return this.remote as unknown as MediaStreamTrack;
  }
  private setNativePaused(paused: boolean): void {
    if (this.closed) return;
    void invokeNative("media_consumer_pause", {
      consumer: this.handle,
      paused,
    }).catch(report);
  }
  pause(): void {
    this.paused = true;
    this.setNativePaused(true);
  }
  resume(): void {
    this.paused = false;
    this.setNativePaused(false);
  }
  async getStats(): Promise<unknown[]> {
    const stats = await invokeNative<{ rtc?: unknown[] }>(
      "media_consumer_stats",
      { consumer: this.handle },
    );
    return Array.isArray(stats.rtc) ? stats.rtc : [];
  }
  close(): void {
    if (this.closed) return;
    this.closed = true;
    // Like mediasoup-client: the track stops without an `ended` event.
    this.remote.readyState = "ended";
    void invokeNative("media_consumer_close", { consumer: this.handle }).catch(
      report,
    );
  }
}

export class NativeTransport extends Emitter {
  readonly id: string;
  closed = false;
  connectionState: RTCPeerConnectionState = "new";
  readonly appData: Record<string, unknown> = {};
  private handle: Promise<number>;
  private producing: Promise<unknown> = Promise.resolve();
  private produceAppData: Record<string, unknown> | undefined;

  constructor(
    device: NativeDevice,
    readonly direction: "send" | "recv",
    options: NativeTransportOptions,
  ) {
    super();
    this.id = options.id;
    const params = {
      id: options.id,
      iceParameters: options.iceParameters,
      iceCandidates: options.iceCandidates,
      dtlsParameters: options.dtlsParameters,
      ...(options.iceServers ? { iceServers: options.iceServers } : {}),
      ...(options.iceTransportPolicy
        ? { iceTransportPolicy: options.iceTransportPolicy }
        : {}),
    };
    this.handle = (async () => {
      const events = await nativeBridge().channel<TransportMessage>((message) =>
        this.onMessage(message),
      );
      const created = await invokeNative<{ transport: number }>(
        "media_transport_create",
        {
          device: await device.nativeHandle(),
          direction,
          options: params,
          events,
        },
      );
      if (this.closed) {
        void invokeNative("media_transport_close", {
          transport: created.transport,
        }).catch(report);
        throw closedError();
      }
      return created.transport;
    })();
    // Failures surface on the first operation.
    this.handle.catch(() => undefined);
  }

  private respond(request: number, result: unknown, error?: unknown): void {
    void this.handle
      .then((transport) =>
        invokeNative("media_transport_respond", {
          transport,
          request,
          ...(error === undefined
            ? { result }
            : {
                error: error instanceof Error ? error.message : String(error),
              }),
        }),
      )
      .catch(report);
  }

  private onMessage(message: TransportMessage): void {
    if (message.type === "connectionstatechange") {
      this.connectionState = message.state;
      if (!this.closed) this.emit("connectionstatechange", message.state);
      return;
    }
    if (this.closed) {
      this.respond(message.request, undefined, closedError());
      return;
    }
    const done = (result: unknown = {}) =>
      this.respond(message.request, result ?? {});
    const fail = (error: unknown) =>
      this.respond(message.request, undefined, error ?? "failed");
    if (message.type === "connect") {
      if (
        !this.emit(
          "connect",
          { dtlsParameters: message.dtlsParameters },
          () => done(),
          fail,
        )
      )
        fail("no connect listener");
      return;
    }
    const handled = this.emit(
      "produce",
      {
        kind: message.kind,
        rtpParameters: message.rtpParameters,
        // The page's own appData (objects, functions) never crosses IPC.
        appData: this.produceAppData ?? {},
      },
      ({ id }: { id: string }) => done({ id }),
      fail,
    );
    if (!handled) fail("no produce listener");
  }

  async produce(options: {
    track?: MediaStreamTrack | null;
    encodings?: RTCRtpEncodingParameters[];
    codecOptions?: Record<string, unknown>;
    codec?: RtpCodecCapability;
    appData?: Record<string, unknown>;
  }): Promise<NativeProducer> {
    // One produce at a time: the PRODUCE request carries the pending appData.
    const run = this.producing.then(
      () => this.produceNow(options),
      () => this.produceNow(options),
    );
    this.producing = run.catch(() => undefined);
    return run;
  }

  private async produceNow(options: {
    track?: MediaStreamTrack | null;
    encodings?: RTCRtpEncodingParameters[];
    codecOptions?: Record<string, unknown>;
    codec?: RtpCodecCapability;
    appData?: Record<string, unknown>;
  }): Promise<NativeProducer> {
    if (this.direction !== "send")
      throw new DOMException("not a sending transport", "InvalidStateError");
    if (this.closed) throw closedError();
    const track = options.track;
    if (!isNativeTrack(track) || track.handle.source === undefined)
      throw new TypeError("native producers send native capture tracks only");
    if (track.readyState === "ended")
      throw new DOMException("track ended", "InvalidStateError");
    const transport = await this.handle;
    this.produceAppData = options.appData;
    try {
      const produced = await invokeNative<{
        producer: number;
        id: string;
        rtpParameters: RtpParameters;
      }>("media_produce", {
        transport,
        source: track.handle.source,
        options: {
          ...(options.codec ? { codec: options.codec.mimeType } : {}),
          ...(options.encodings?.length
            ? {
                encodings: options.encodings.map((encoding) => ({
                  ...(encoding.scaleResolutionDownBy !== undefined
                    ? { scaleResolutionDownBy: encoding.scaleResolutionDownBy }
                    : {}),
                  ...(encoding.maxBitrate !== undefined
                    ? { maxBitrate: encoding.maxBitrate }
                    : {}),
                  ...(encoding.maxFramerate !== undefined
                    ? { maxFramerate: encoding.maxFramerate }
                    : {}),
                  ...(encoding.active !== undefined
                    ? { active: encoding.active }
                    : {}),
                })),
              }
            : {}),
          ...(options.codecOptions
            ? { codecOptions: options.codecOptions }
            : {}),
        },
      });
      const producer = new NativeProducer(
        produced.producer,
        produced.id,
        track,
        produced.rtpParameters,
        options.appData,
      );
      await producer.refreshParameters().catch(report);
      if (this.closed) {
        producer.close();
        throw closedError();
      }
      return producer;
    } finally {
      this.produceAppData = undefined;
    }
  }

  async consume(options: {
    id: string;
    producerId: string;
    kind: "audio" | "video";
    rtpParameters: RtpParameters;
    appData?: Record<string, unknown>;
  }): Promise<NativeConsumer> {
    if (this.direction !== "recv")
      throw new DOMException("not a receiving transport", "InvalidStateError");
    if (this.closed) throw closedError();
    const transport = await this.handle;
    const consumed = await invokeNative<{ consumer: number }>("media_consume", {
      transport,
      params: {
        id: options.id,
        producerId: options.producerId,
        kind: options.kind,
        rtpParameters: options.rtpParameters,
      },
    });
    const consumer = new NativeConsumer(
      consumed.consumer,
      options.id,
      options.producerId,
      options.kind,
      options.rtpParameters,
      options.appData ?? {},
    );
    if (this.closed) {
      consumer.close();
      throw closedError();
    }
    return consumer;
  }

  async restartIce({
    iceParameters,
  }: {
    iceParameters: unknown;
  }): Promise<void> {
    const transport = await this.handle;
    await invokeNative("media_transport_restart_ice", {
      transport,
      iceParameters,
    });
  }

  async getStats(): Promise<unknown[]> {
    const transport = await this.handle;
    return invokeNative<unknown[]>("media_transport_stats", { transport });
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    this.connectionState = "closed";
    void this.handle
      .then((transport) => invokeNative("media_transport_close", { transport }))
      .catch(() => undefined);
  }
}

export class NativeDevice {
  readonly handlerName = "GelabberNative";
  loaded = false;
  private handle: number | null = null;
  private capabilities: RtpCapabilities = { codecs: [], headerExtensions: [] };
  private producible = { audio: false, video: false };

  async load({
    routerRtpCapabilities,
  }: {
    routerRtpCapabilities: RtpCapabilities;
  }): Promise<void> {
    if (this.loaded)
      throw new DOMException("already loaded", "InvalidStateError");
    const loaded = await invokeNative<{
      device: number;
      rtpCapabilities: RtpCapabilities;
      canProduce: { audio: boolean; video: boolean };
    }>("media_device_load", { capabilities: routerRtpCapabilities });
    this.handle = loaded.device;
    this.capabilities = loaded.rtpCapabilities;
    this.producible = loaded.canProduce;
    this.loaded = true;
  }
  async nativeHandle(): Promise<number> {
    if (this.handle === null)
      throw new DOMException("device not loaded", "InvalidStateError");
    return this.handle;
  }
  /** libmediasoupclient keeps one capability set for both directions. */
  get rtpCapabilities(): RtpCapabilities {
    return this.capabilities;
  }
  get recvRtpCapabilities(): RtpCapabilities {
    return this.capabilities;
  }
  get sendRtpCapabilities(): RtpCapabilities {
    return this.capabilities;
  }
  canProduce(kind: "audio" | "video"): boolean {
    return this.producible[kind];
  }
  createSendTransport(options: NativeTransportOptions): NativeTransport {
    return new NativeTransport(this, "send", options);
  }
  createRecvTransport(options: NativeTransportOptions): NativeTransport {
    return new NativeTransport(this, "recv", options);
  }
  /** Releases the app's handle; open transports keep the native device. */
  close(): void {
    const handle = this.handle;
    this.handle = null;
    if (handle !== null)
      void invokeNative("media_device_close", { device: handle }).catch(
        () => undefined,
      );
  }
}

/** For MediasoupConnection's `deviceFactory`. */
export async function createNativeDevice(): Promise<Device> {
  return new NativeDevice() as unknown as Device;
}
