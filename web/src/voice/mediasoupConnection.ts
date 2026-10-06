import { Device } from "mediasoup-client";
import type {
  Consumer,
  Producer,
  ProducerCodecOptions,
  ProducerOptions,
  RtpCapabilities,
  Transport,
  TransportOptions,
} from "mediasoup-client/types";
import type { TrackKind } from "../ws/protocol.ts";
import {
  MediaError,
  type ConsumerAnnouncement,
  type IceServer,
  type MediaEvent,
  type MediaRequest,
} from "./media.ts";
import { statsEntriesFromReport, type StatsEntry } from "./diagnostics.ts";
import type { MediaPriority } from "./mediaPriority.ts";
import { ViewerLayerController } from "./viewerLayers.ts";

export type RtpSenderParameters = {
  encodings: Array<{
    maxBitrate?: number;
    maxFramerate?: number;
    priority?: MediaPriority;
    networkPriority?: MediaPriority;
  }>;
  transactionId?: string;
};
export type MediaSender = {
  readonly sourceKind: TrackKind;
  readonly producerId: string;
  readonly epoch: string;
  readonly track: MediaStreamTrack | null;
  readonly rtpSender: RTCRtpSender | undefined;
  replaceTrack(track: MediaStreamTrack | null): Promise<void>;
  getParameters(): RtpSenderParameters;
  setParameters(params: RtpSenderParameters): Promise<void>;
};
export type MediaPublication = {
  kind: TrackKind;
  track: MediaStreamTrack;
  streamId: string;
  epoch: string;
  lc?: string;
  parent?: string;
  paused?: boolean;
  /** Transient local operation controls; never part of the product wire. */
  deadlineEpochMs?: number;
  isCurrent?: () => boolean;
};
export type ReceivedSource = ConsumerAnnouncement & {
  consumer: Consumer;
  track: MediaStreamTrack;
  stream: MediaStream;
  rtpReceiver?: RTCRtpReceiver;
  layers: { spatial: number | null; temporal: number | null };
};
export type MediaConnectionOptions = {
  role: "voice" | "watch";
  generation: string;
  iceServers: IceServer[];
  request: MediaRequest;
  codecOptions(kind: TrackKind): ProducerCodecOptions;
  onConsumer(source: ReceivedSource): void;
  onConsumerClosed(source: ReceivedSource): void;
  onTransportState(direction: "send" | "recv", state: string): void;
  onError(error: unknown): void;
  deviceFactory?: () => Promise<Device>;
  /** Test injection; production SDK operations are bounded to ten seconds. */
  sdkTimeoutMs?: number;
};
export interface MediaConnection {
  start(capabilities: RtpCapabilities): Promise<void>;
  publish(publication: MediaPublication): Promise<MediaSender>;
  closeSource(kind: TrackKind, expectedProducerId?: string): Promise<void>;
  setSourcePaused(kind: TrackKind, paused: boolean): Promise<void>;
  sender(kind: TrackKind): MediaSender | undefined;
  senders(): MediaSender[];
  consumers(): ReceivedSource[];
  transportState(direction: "send" | "recv"): string;
  restartIce(direction: "send" | "recv"): Promise<void>;
  getStats(): Promise<StatsEntry[]>;
  handleEvent(event: MediaEvent): void;
  close(): void;
}
export const createMediaConnection = (
  options: MediaConnectionOptions,
): MediaConnection => new MediasoupConnection(options);

const consumerKey = (source: { consumerId: string; generation: string }) =>
  `${source.consumerId}:${source.generation}`;

type Publication = {
  producer: Producer;
  sender: MediaSender;
  input: MediaPublication;
  codec: string;
  paused: boolean;
};
function emitMediaHook(name: string, detail: unknown): void {
  if (typeof window !== "undefined" && typeof CustomEvent !== "undefined")
    window.dispatchEvent(new CustomEvent(name, { detail }));
}
function sdkErrorName(error: unknown): string {
  if (typeof error !== "object" || error === null) return "";
  // Browser DOMException and errors from another realm need not inherit this
  // realm's Error. Keep their public name without inspecting SDK internals.
  try {
    const name = (error as { name?: unknown }).name;
    return typeof name === "string" ? name : "";
  } catch {
    return "";
  }
}

/** Public SDK surface only; capture and privacy remain owned by the session. */
export class MediasoupConnection implements MediaConnection {
  private device: Device | null = null;
  private send: Transport | null = null;
  private recv: Transport | null = null;
  private ownedTransports = new Set<Transport>();
  private sendCreating: Promise<Transport> | null = null;
  private closed = false;
  private publications = new Map<TrackKind, Publication>();
  private received = new Map<string, ReceivedSource>();
  private removed = new Set<string>();
  private queued: MediaEvent[] = [];
  private sendChain: Promise<unknown> = Promise.resolve();
  private sendPending = 0;
  private consumePending = new Set<string>();
  private sendUses = 0;
  private recvUses = 0;
  private compactingSend = false;
  private consumeChain: Promise<void> = Promise.resolve();
  private layerTimer: ReturnType<typeof setInterval> | null = null;
  private layers = new ViewerLayerController();
  private samplingLayers = false;
  private sdkWaits = new Set<() => void>();
  private sendDeadlineEpochMs: number | undefined;
  constructor(private readonly options: MediaConnectionOptions) {}

  private live(): void {
    if (this.closed) throw new MediaError("connection_closed");
  }
  private async sdk<T>(
    operation: () => Promise<T>,
    direction: "send" | "recv",
    fatal = true,
    retire?: (value: T) => void,
    deadlineEpochMs?: number,
  ): Promise<T> {
    this.live();
    if (this.sdkWaits.size >= 128) throw new MediaError("request_overflow");
    const budget = Math.max(
      0,
      Math.min(
        this.options.sdkTimeoutMs ?? 10_000,
        (deadlineEpochMs ??
          (direction === "send" ? this.sendDeadlineEpochMs : undefined) ??
          Infinity) - Date.now(),
      ),
    );
    const deadline = performance.now() + budget;
    let finished = false;
    return new Promise<T>((resolve, reject) => {
      const settleError = (error: unknown) => {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        this.sdkWaits.delete(cancel);
        reject(error);
      };
      const timeout = () => {
        const error = new MediaError("sdk_timeout");
        settleError(error);
        if (fatal) this.fail(error, direction);
      };
      const cancel = () => settleError(new MediaError("connection_closed"));
      const timer = setTimeout(timeout, budget);
      this.sdkWaits.add(cancel);
      // Check elapsed time around the SDK call too: a blocked event loop must
      // not turn a late resolution into success before the timer can run.
      void Promise.resolve()
        .then(() => {
          if (finished || this.closed)
            throw new MediaError("connection_closed");
          if (performance.now() >= deadline)
            throw new MediaError("sdk_timeout");
          return operation();
        })
        .then(
          (value) => {
            if (finished || this.closed) {
              retire?.(value);
              return;
            }
            if (performance.now() >= deadline) {
              retire?.(value);
              timeout();
              return;
            }
            finished = true;
            clearTimeout(timer);
            this.sdkWaits.delete(cancel);
            resolve(value);
          },
          (error: unknown) => {
            if (finished) return;
            const failure = sdkErrorName(error)
              ? error
              : new MediaError("sdk_failed");
            if (failure instanceof MediaError && failure.code === "sdk_timeout")
              timeout();
            else settleError(failure);
          },
        );
    });
  }
  async start(capabilities: RtpCapabilities): Promise<void> {
    this.live();
    const device = await this.sdk(
      () => this.options.deviceFactory?.() ?? Device.factory(),
      "recv",
    );
    this.live();
    await this.sdk(
      () => device.load({ routerRtpCapabilities: capabilities }),
      "recv",
    );
    this.live();
    this.device = device;
    await this.options.request("capabilities", {
      rtp: device.recvRtpCapabilities,
    });
    this.live();
    this.recv = await this.makeTransport("recv");
    this.live();
    for (const event of this.queued.splice(0)) this.handleEvent(event);
    this.layerTimer = setInterval(() => {
      void this.sampleLayers();
    }, 2000);
  }

  private async makeTransport(direction: "send" | "recv"): Promise<Transport> {
    const params = await this.options.request(
      "transport",
      { direction },
      direction === "send" ? this.sendDeadlineEpochMs : undefined,
    );
    this.live();
    if (
      !this.device ||
      !params.id ||
      !params.iceParameters ||
      !Array.isArray(params.iceCandidates) ||
      !params.dtlsParameters
    )
      throw new MediaError("invalid_transport");
    const options: TransportOptions = {
      ...params,
      iceServers: this.options.iceServers,
    };
    const transport =
      direction === "send"
        ? this.device.createSendTransport(options)
        : this.device.createRecvTransport(options);
    this.ownedTransports.add(transport);
    transport.on("connect", ({ dtlsParameters }, done, fail) => {
      void this.options
        .request(
          "connect",
          { transportId: transport.id, dtls: dtlsParameters },
          direction === "send" ? this.sendDeadlineEpochMs : undefined,
        )
        .then(() => {
          this.live();
          done();
        }, fail)
        .catch(fail);
    });
    transport.on("connectionstatechange", (state) => {
      if (!this.closed) this.options.onTransportState(direction, state);
    });
    if (direction === "send")
      transport.on("produce", ({ rtpParameters, appData }, done, fail) => {
        const publication = appData.publication as MediaPublication;
        const old = appData.expectedOldProducerId as string | undefined;
        try {
          this.checkPublication(publication);
        } catch (error) {
          fail(error as Error);
          return;
        }
        void this.options
          .request(
            "produce",
            {
              k: publication.kind,
              rtp: rtpParameters,
              epoch: publication.epoch,
              ...(publication.parent ? { parent: publication.parent } : {}),
              ...(publication.lc ? { lc: publication.lc } : {}),
              ...(old ? { expectedOldProducerId: old } : {}),
              ...(publication.track.kind === "video"
                ? {
                    height: Math.min(
                      65535,
                      publication.track.getSettings?.().height ?? 0,
                    ),
                  }
                : {}),
              paused: publication.paused ?? false,
            },
            publication.deadlineEpochMs ?? this.sendDeadlineEpochMs,
          )
          .then(({ producerId }) => {
            if (!producerId) throw new MediaError("invalid_producer");
            if (
              this.closed ||
              publication.isCurrent?.() === false ||
              Date.now() >=
                (publication.deadlineEpochMs ??
                  this.sendDeadlineEpochMs ??
                  Infinity)
            ) {
              void this.options
                .request("closeProducer", { producerId })
                .catch(() => {});
              throw new MediaError("connection_closed");
            }
            done({ id: producerId });
          }, fail)
          .catch(fail);
      });
    return transport;
  }

  private async sendTransport(): Promise<Transport> {
    this.live();
    if (this.options.role === "watch") throw new MediaError("watch_read_only");
    if (this.send) return this.send;
    this.sendCreating ??= this.makeTransport("send")
      .then((transport) => {
        this.live();
        this.send = transport;
        return transport;
      })
      .finally(() => {
        this.sendCreating = null;
      });
    return this.sendCreating;
  }
  private serialize<T>(_kind: TrackKind, job: () => Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(new MediaError("connection_closed"));
    if (this.sendPending >= 64)
      return Promise.reject(new MediaError("request_overflow"));
    this.sendPending += 1;
    const run = this.sendChain.then(job, job).finally(() => {
      this.sendPending -= 1;
    });
    this.sendChain = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  }
  publish(input: MediaPublication): Promise<MediaSender> {
    return this.serialize(input.kind, async () => {
      const previousDeadline = this.sendDeadlineEpochMs;
      this.sendDeadlineEpochMs = input.deadlineEpochMs;
      try {
        return await this.publishNow(input);
      } finally {
        this.sendDeadlineEpochMs = previousDeadline;
      }
    });
  }
  private checkPublication(input: MediaPublication): void {
    this.live();
    if (input.isCurrent?.() === false)
      throw new MediaError("publication_cancelled");
    if (
      Date.now() >=
      (input.deadlineEpochMs ?? this.sendDeadlineEpochMs ?? Infinity)
    )
      throw new MediaError("live_recovery_timeout");
  }
  private async publishNow(input: MediaPublication): Promise<MediaSender> {
    this.checkPublication(input);
    const device = this.device;
    if (!device || !device.canProduce(input.track.kind as "audio" | "video"))
      throw new MediaError("unsupported_codec");
    if ((input.kind === "sa" || input.kind === "la") && !input.parent)
      throw new MediaError("missing_parent");
    const codecOptions = {
      ...this.options.codecOptions(input.kind),
      // Public SDK hint for the initial libwebrtc video bandwidth estimate.
      // It sets no minimum/maximum bitrate or per-encoding upload limit.
      ...(input.track.kind === "video"
        ? { videoGoogleStartBitrate: 1000 }
        : {}),
    };
    const signature = JSON.stringify(codecOptions);
    let previous = this.publications.get(input.kind);
    if (
      previous &&
      previous.codec === signature &&
      previous.input.epoch === input.epoch &&
      previous.input.lc === input.lc &&
      previous.input.parent === input.parent
    ) {
      if (previous.producer.track !== input.track)
        await this.sdk(
          () => previous!.producer.replaceTrack({ track: input.track }),
          "send",
        );
      this.checkPublication(input);
      previous.input = {
        ...input,
        deadlineEpochMs: undefined,
        isCurrent: undefined,
      };
      emitMediaHook("gelabber:media-producer", {
        producerId: previous.producer.id,
        k: input.kind,
        epoch: input.epoch,
        track: previous.producer.track,
        sender: previous.producer.rtpSender,
        encodings: previous.producer.rtpParameters.encodings,
      });
      await this.pauseNow(input.kind, input.paused ?? false);
      return previous.sender;
    }
    if (
      !this.compactingSend &&
      this.send &&
      this.sendUses >= Math.max(4, this.publications.size + 1)
    ) {
      await this.compactSend();
      previous = this.publications.get(input.kind);
    }
    if (input.kind === "sa" || input.kind === "la") {
      const parent = this.publications.get(input.kind === "sa" ? "s" : "l");
      if (!parent || parent.input.epoch !== input.epoch)
        throw new MediaError("missing_parent");
      input = { ...input, parent: parent.producer.id };
    }
    const transport = await this.sendTransport();
    this.checkPublication(input);
    const codec =
      input.track.kind === "video"
        ? (device.sendRtpCapabilities.codecs?.find(
            (c) => c.mimeType.toLowerCase() === "video/vp8",
          ) ??
          device.sendRtpCapabilities.codecs?.find(
            (c) => c.mimeType.toLowerCase() === "video/h264",
          ) ??
          device.sendRtpCapabilities.codecs?.find(
            (c) => c.mimeType.toLowerCase() === "video/vp9",
          ) ??
          device.sendRtpCapabilities.codecs?.find(
            (c) => c.mimeType.toLowerCase() === "video/av1",
          ))
        : device.sendRtpCapabilities.codecs?.find(
            (c) => c.mimeType.toLowerCase() === "audio/opus",
          );
    if (!codec) throw new MediaError("unsupported_codec");
    const vp8 = codec.mimeType.toLowerCase() === "video/vp8";
    const base = {
      track: input.track,
      streamId: input.streamId,
      codec,
      codecOptions,
      stopTracks: false,
      disableTrackOnPause: false,
      appData: {
        publication: input,
        expectedOldProducerId: previous?.producer.id,
      },
    };
    const produce = async (options: ProducerOptions): Promise<Producer> => {
      try {
        return await this.sdk(
          () => transport.produce(options),
          "send",
          true,
          (late) => late.close(),
          input.deadlineEpochMs,
        );
      } catch (error) {
        if (
          input.deadlineEpochMs !== undefined &&
          error instanceof MediaError &&
          ["sdk_timeout", "request_timeout", "live_recovery_timeout"].includes(
            error.code,
          )
        ) {
          // A timed-out RPC can hide a server-created producer ID. Stop the
          // known owned public transport and its server publications regardless
          // of whether the late produce callback ever returns that ID.
          if (!transport.closed) transport.close();
          void this.options
            .request("closeTransport", { transportId: transport.id })
            .catch(() => {});
          this.fail(error, "send");
        }
        // A native SDP rejection can leave the SDK handler between descriptions.
        // Retire its public send transport before a later UI retry, retaining
        // existing captures and the receive transport. Also covers VP8 fallback.
        if (
          sdkErrorName(error) === "OperationError" &&
          !this.compactingSend &&
          this.send === transport
        )
          await this.compactSend();
        throw error;
      }
    };
    let producer: Producer;
    this.sendUses += 1;
    try {
      producer = await produce({
        ...base,
        ...(vp8
          ? {
              encodings: [
                { scaleResolutionDownBy: 4 },
                { scaleResolutionDownBy: 1 },
              ],
            }
          : {}),
      });
    } catch (error) {
      const name = sdkErrorName(error);
      if (!vp8 || (name !== "UnsupportedError" && name !== "NotSupportedError"))
        throw error;
      this.live();
      this.sendUses += 1;
      producer = await produce(base);
    }
    if (
      this.closed ||
      input.isCurrent?.() === false ||
      Date.now() >=
        (input.deadlineEpochMs ?? this.sendDeadlineEpochMs ?? Infinity)
    ) {
      producer.close();
      void this.options
        .request("closeProducer", { producerId: producer.id })
        .catch(() => {});
      throw new MediaError(
        this.closed ? "connection_closed" : "publication_cancelled",
      );
    }
    const sender: MediaSender = {
      sourceKind: input.kind,
      get producerId() {
        return held.producer.id;
      },
      get epoch() {
        return held.input.epoch;
      },
      get track() {
        return held.producer.track;
      },
      get rtpSender() {
        return held.producer.rtpSender;
      },
      replaceTrack: async (track) => {
        this.live();
        if (this.publications.get(input.kind) !== held)
          throw new MediaError("publication_replaced");
        await this.sdk(() => held.producer.replaceTrack({ track }), "send");
        this.live();
        if (track) held.input = { ...held.input, track };
      },
      getParameters: () =>
        held.producer.rtpSender?.getParameters() ?? { encodings: [] },
      setParameters: async (params) => {
        this.live();
        const sender = held.producer.rtpSender;
        if (sender)
          await this.sdk(
            () => sender.setParameters(params as RTCRtpSendParameters),
            "send",
          );
        this.live();
      },
    };
    const held: Publication = {
      producer,
      sender,
      input: { ...input, deadlineEpochMs: undefined, isCurrent: undefined },
      codec: signature,
      paused: input.paused ?? false,
    };
    this.publications.set(input.kind, held);
    emitMediaHook("gelabber:media-producer", {
      producerId: producer.id,
      k: input.kind,
      epoch: input.epoch,
      track: producer.track,
      sender: producer.rtpSender,
      encodings: producer.rtpParameters.encodings,
    });
    // The server transaction already replaced the old producer. The app owns tracks.
    previous?.producer.close();
    await this.pauseNow(input.kind, input.paused ?? false);
    return sender;
  }

  /** The pinned Firefox handler does not reuse stopped m-lines. Compact through
   * public transports, closing the old native PC before creating its replacement. */
  private async compactSend(): Promise<void> {
    const old = this.send;
    if (!old) return;
    const inputs = [...this.publications.values()]
      .flatMap((p) =>
        p.producer.track ? [{ ...p.input, track: p.producer.track }] : [],
      )
      .sort(
        (a, b) =>
          Number(a.kind === "sa" || a.kind === "la") -
          Number(b.kind === "sa" || b.kind === "la"),
      );
    this.compactingSend = true;
    this.publications.clear();
    this.send = null;
    old.close();
    this.ownedTransports.delete(old);
    this.sendUses = 0;
    try {
      await this.options.request(
        "closeTransport",
        { transportId: old.id },
        this.sendDeadlineEpochMs,
      );
      this.live();
      for (const input of inputs) await this.publishNow(input);
    } catch (error) {
      if (!this.closed) this.options.onTransportState("send", "failed");
      throw error;
    } finally {
      this.compactingSend = false;
    }
  }
  private async compactRecv(): Promise<void> {
    const old = this.recv;
    if (!old) return;
    this.recv = null;
    for (const [id, source] of [...this.received]) {
      this.rememberClosed(source);
      this.removeConsumer(id);
    }
    old.close();
    this.ownedTransports.delete(old);
    this.recvUses = 0;
    try {
      await this.options.request("closeTransport", { transportId: old.id });
      this.live();
      this.recv = await this.makeTransport("recv");
      this.live();
      // Server creates fresh authorized Consumers for the new receive transport.
      for (const event of this.queued.splice(0)) this.handleEvent(event);
    } catch (error) {
      if (!this.closed) this.options.onTransportState("recv", "failed");
      throw error;
    }
  }

  private async pauseNow(kind: TrackKind, paused: boolean): Promise<void> {
    this.live();
    const held = this.publications.get(kind);
    if (!held) return;
    if (paused) held.producer.pause();
    else held.producer.resume();
    if (held.paused === paused) return;
    await this.options.request(
      paused ? "pauseProducer" : "resumeProducer",
      {
        producerId: held.producer.id,
      },
      this.sendDeadlineEpochMs,
    );
    this.live();
    if (this.publications.get(kind) === held) {
      held.paused = paused;
      held.input = { ...held.input, paused };
    }
  }
  setSourcePaused(kind: TrackKind, paused: boolean): Promise<void> {
    return this.serialize(kind, () => this.pauseNow(kind, paused));
  }
  closeSource(kind: TrackKind, expectedProducerId?: string): Promise<void> {
    return this.serialize(kind, async () => {
      const held = this.publications.get(kind);
      if (
        !held ||
        (expectedProducerId !== undefined &&
          held.producer.id !== expectedProducerId)
      )
        return;
      this.publications.delete(kind);
      held.producer.close();
      await this.options.request("closeProducer", {
        producerId: held.producer.id,
      });
    });
  }
  sender(kind: TrackKind): MediaSender | undefined {
    return this.publications.get(kind)?.sender;
  }
  senders(): MediaSender[] {
    return [...this.publications.values()].map((p) => p.sender);
  }
  consumers(): ReceivedSource[] {
    return [...this.received.values()];
  }
  transportState(direction: "send" | "recv"): string {
    return (
      (direction === "send" ? this.send : this.recv)?.connectionState ?? "new"
    );
  }
  async restartIce(direction: "send" | "recv"): Promise<void> {
    this.live();
    const transport = direction === "send" ? this.send : this.recv;
    if (!transport) return;
    const params = await this.options.request("restartIce", {
      transportId: transport.id,
    });
    this.live();
    await this.sdk(() => transport.restartIce(params), direction);
    this.live();
  }

  private fail(error: Error, direction: "send" | "recv"): void {
    if (this.closed) return;
    this.options.onError(error);
    this.options.onTransportState(direction, "failed");
    this.close();
  }
  private rememberClosed(source: {
    consumerId: string;
    generation: string;
  }): void {
    this.removed.add(consumerKey(source));
    while (this.removed.size > 256) {
      const candidate = [...this.removed].find(
        (key) => !this.consumePending.has(key),
      );
      if (!candidate) break;
      this.removed.delete(candidate);
    }
  }

  handleEvent(event: MediaEvent): void {
    if (this.closed) return;
    if (event.op === "producerClosed") {
      for (const [kind, held] of this.publications)
        if (
          held.producer.id === event.producerId &&
          held.input.epoch === event.epoch
        ) {
          this.publications.delete(kind);
          held.producer.close();
        }
      return;
    }
    if (event.op === "consumerClosed") {
      this.rememberClosed(event);
      this.queued = this.queued.filter(
        (queued) =>
          !("consumerId" in queued) ||
          consumerKey(queued) !== consumerKey(event),
      );
      const held = this.received.get(event.consumerId);
      if (held?.generation === event.generation)
        this.removeConsumer(event.consumerId);
      return;
    }
    if ("consumerId" in event && this.removed.has(consumerKey(event))) return;
    if (!this.recv) {
      if (this.queued.length < 64) this.queued.push(event);
      else this.fail(new MediaError("event_overflow"), "recv");
      return;
    }
    if (event.op === "consumer") {
      const key = consumerKey(event);
      if (
        this.consumePending.has(key) ||
        this.received.has(event.consumerId) ||
        this.removed.has(key)
      )
        return;
      if (this.consumePending.size >= 64) {
        this.fail(new MediaError("event_overflow"), "recv");
        return;
      }
      this.consumePending.add(key);
      const job = () => this.consume(event);
      this.consumeChain = this.consumeChain.then(job, job).finally(() => {
        this.consumePending.delete(key);
      });
      return;
    }
    const held = this.received.get(event.consumerId);
    if (!held) {
      if (this.queued.length < 64) this.queued.push(event);
      else this.fail(new MediaError("event_overflow"), "recv");
      return;
    }
    if (held.generation !== event.generation) return;
    if (event.op === "consumerState") {
      held.paused = event.paused;
      if (event.paused) held.consumer.pause();
      else held.consumer.resume();
    }
    if (event.op === "layers") {
      held.layers = {
        spatial: event.spatialLayer,
        temporal: event.temporalLayer,
      };
      emitMediaHook("gelabber:media-layers", {
        ...event,
        producerId: held.producerId,
        owner: held.owner,
        k: held.k,
        epoch: held.epoch,
      });
    }
  }
  private async consume(event: ConsumerAnnouncement): Promise<void> {
    if (
      this.closed ||
      this.removed.has(consumerKey(event)) ||
      this.received.has(event.consumerId)
    )
      return;
    let consumer: Consumer | null = null;
    const transport = this.recv;
    if (!transport) return;
    try {
      if (this.recvUses >= Math.max(4, this.received.size + 1)) {
        this.rememberClosed(event);
        await this.compactRecv();
        return;
      }
      this.recvUses += 1;
      consumer = await this.sdk(
        () =>
          transport.consume({
            id: event.consumerId,
            producerId: event.producerId,
            kind: event.kind,
            rtpParameters: event.rtpParameters,
            appData: {
              owner: event.owner,
              sourceKind: event.k,
              epoch: event.epoch,
              generation: event.generation,
              parent: event.parent,
            },
          }),
        "recv",
        true,
        (late) => late.close(),
      );
      if (
        this.closed ||
        this.recv !== transport ||
        this.removed.has(consumerKey(event))
      ) {
        consumer.close();
        return;
      }
      const source: ReceivedSource = {
        ...event,
        consumer,
        track: consumer.track,
        stream: new MediaStream([consumer.track]),
        rtpReceiver: consumer.rtpReceiver,
        layers: { spatial: null, temporal: null },
      };
      this.received.set(event.consumerId, source);
      if (event.paused) consumer.pause();
      emitMediaHook("gelabber:media-consumer", {
        consumerId: source.consumerId,
        producerId: source.producerId,
        owner: source.owner,
        k: source.k,
        epoch: source.epoch,
        generation: source.generation,
        track: source.track,
        receiver: source.rtpReceiver,
        rtpParameters: consumer.rtpParameters,
      });
      this.options.onConsumer(source);
      if (
        this.closed ||
        this.recv !== transport ||
        this.removed.has(consumerKey(event)) ||
        this.received.get(event.consumerId) !== source
      ) {
        this.removeConsumer(event.consumerId);
        return;
      }
      await this.options.request("consumerReady", {
        consumerId: event.consumerId,
        generation: event.generation,
      });
      if (this.closed || this.removed.has(consumerKey(event))) {
        this.removeConsumer(event.consumerId);
        return;
      }
      const queued = this.queued.filter(
        (e) =>
          "consumerId" in e &&
          e.consumerId === event.consumerId &&
          e.generation === event.generation,
      );
      this.queued = this.queued.filter(
        (e) =>
          !("consumerId" in e) ||
          e.consumerId !== event.consumerId ||
          e.generation !== event.generation,
      );
      for (const pending of queued) this.handleEvent(pending);
    } catch (error) {
      consumer?.close();
      this.removeConsumer(event.consumerId);
      if (!this.closed) {
        await this.options
          .request("consumerFailed", {
            consumerId: event.consumerId,
            generation: event.generation,
          })
          .catch(() => {});
        this.options.onError(error);
      }
    }
  }
  private removeConsumer(id: string): void {
    const source = this.received.get(id);
    if (!source) return;
    this.received.delete(id);
    source.consumer.close();
    this.options.onConsumerClosed(source);
  }
  async getStats(): Promise<StatsEntry[]> {
    this.live();
    const scopes: Array<{
      transport: Transport;
      direction: "send" | "recv";
      report: () => Promise<RTCStatsReport>;
    }> = [];
    if (this.send) {
      const transport = this.send;
      scopes.push({
        transport,
        direction: "send",
        report: () => transport.getStats(),
      });
      for (const { producer } of this.publications.values())
        scopes.push({
          transport,
          direction: "send",
          report: () => producer.getStats(),
        });
    }
    if (this.recv) {
      const transport = this.recv;
      scopes.push({
        transport,
        direction: "recv",
        report: () => transport.getStats(),
      });
      for (const { consumer } of this.received.values())
        scopes.push({
          transport,
          direction: "recv",
          report: () => consumer.getStats(),
        });
    }
    const reports = await Promise.all(
      scopes.map(async ({ transport, direction, report }) => ({
        id: transport.id,
        rows: statsEntriesFromReport(await this.sdk(report, direction, false)),
      })),
    );
    this.live();
    const qualified = new Map<string, StatsEntry>();
    for (const { id, rows } of reports)
      for (const entry of rows) {
        const row: StatsEntry & Record<string, unknown> = {
          ...entry,
          id: `${id}:${entry.id}`,
        };
        for (const [key, value] of Object.entries(row))
          if (key !== "id" && key.endsWith("Id") && typeof value === "string")
            row[key] = `${id}:${value}`;
        qualified.set(row.id, row);
      }
    return [...qualified.values()];
  }
  private async sampleLayers(): Promise<void> {
    if (this.closed || this.samplingLayers) return;
    this.samplingLayers = true;
    try {
      const sources = this.consumers().filter((s) => s.kind === "video");
      const rows = (
        await Promise.all(
          sources.map(async (s) =>
            statsEntriesFromReport(
              await this.sdk(() => s.consumer.getStats(), "recv", false),
            ),
          ),
        )
      ).flat();
      if (this.closed) return;
      this.layers.update(
        rows,
        sources.map((s) => ({
          consumerId: s.consumerId,
          generation: s.generation,
          trackId: s.track.id,
          ssrc: s.consumer.rtpParameters.encodings?.[0]?.ssrc,
        })),
        (data) => {
          void this.options.request("q", data).catch(this.options.onError);
        },
      );
    } catch {
      /* Missing browser stats do not invent congestion. */
    } finally {
      this.samplingLayers = false;
    }
  }
  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const cancel of [...this.sdkWaits]) cancel();
    this.sdkWaits.clear();
    if (this.layerTimer) clearInterval(this.layerTimer);
    this.layerTimer = null;
    for (const held of this.publications.values()) held.producer.close();
    this.publications.clear();
    for (const id of [...this.received.keys()]) this.removeConsumer(id);
    for (const transport of this.ownedTransports) transport.close();
    this.ownedTransports.clear();
    this.send = null;
    this.recv = null;
    this.queued = [];
    this.sendChain = Promise.resolve();
  }
}
