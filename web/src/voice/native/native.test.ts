import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { RtpCapabilities, RtpParameters } from "mediasoup-client/types";
import { captureMicrophone } from "../audioProcessing.ts";
import type { MediaMethod, MediaRequest } from "../media.ts";
import {
  createMediaConnection,
  type ReceivedSource,
} from "../mediasoupConnection.ts";
import { listMediaDevices, useMediaSettings } from "../settings.ts";
import { setNativeBridgeForTests, type NativeBridge } from "./bridge.ts";
import { nativeGetDisplayMedia } from "./capture.ts";
import {
  NativeAudioOutput,
  NativeStream,
  NativeTrack,
  createAudioOutput,
  createStream,
  isNativeTrack,
  resetNativeOutputForTests,
} from "./tracks.ts";

const GENERATION = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const EPOCH = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
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
const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

type Call = { command: string; args: Record<string, unknown> };
type Message = Record<string, unknown>;

/** The desktop app's media commands with the core's request/response flow:
 * the first produce/consume on a transport asks for `connect`, produce asks
 * for `produce`, both wait for media_transport_respond. */
class FakeCore implements NativeBridge {
  calls: Call[] = [];
  screenStates: Array<Record<string, unknown>> = [];
  devices = {
    inputs: [
      { id: "", name: "default: Headset" },
      { id: "Headset", name: "Headset" },
    ],
    outputs: [
      { id: "", name: "default: Speakers" },
      { id: "Speakers", name: "Speakers" },
    ],
    input: "",
    output: "",
  };
  private next = 0;
  private request = 0;
  private channels = new Map<number, (message: Message) => void>();
  private connected = new Set<number>();
  private waiting = new Map<number, (answer: Message) => void>();

  async channel<T>(onMessage: (message: T) => void) {
    return { onMessage } as unknown;
  }
  calledWith(command: string): Array<Record<string, unknown>> {
    return this.calls.filter((c) => c.command === command).map((c) => c.args);
  }
  private ask(transport: number, message: Message): Promise<Message> {
    const request = ++this.request;
    return new Promise((resolve) => {
      this.waiting.set(request, resolve);
      this.channels.get(transport)!({ ...message, request });
    });
  }
  private async connect(transport: number): Promise<void> {
    if (this.connected.has(transport)) return;
    this.connected.add(transport);
    const answer = await this.ask(transport, {
      type: "connect",
      dtlsParameters: { role: "client", fingerprints: [] },
    });
    if (answer.error) throw new Error(String(answer.error));
    this.channels.get(transport)!({
      type: "connectionstatechange",
      state: "connected",
    });
  }
  async invoke<T>(command: string, args: Record<string, unknown> = {}) {
    this.calls.push({ command, args });
    const answer = async (): Promise<unknown> => {
      switch (command) {
        case "media_device_load":
          return {
            device: ++this.next,
            rtpCapabilities: args.capabilities,
            canProduce: { audio: true, video: true },
          };
        case "media_transport_create": {
          const transport = ++this.next;
          const events = args.events as {
            onMessage: (message: Message) => void;
          };
          this.channels.set(transport, events.onMessage);
          return { transport, id: (args.options as { id: string }).id };
        }
        case "media_transport_respond": {
          const resolve = this.waiting.get(args.request as number);
          this.waiting.delete(args.request as number);
          resolve?.(args);
          return null;
        }
        case "media_produce": {
          const transport = args.transport as number;
          await this.connect(transport);
          const answer = await this.ask(transport, {
            type: "produce",
            kind: "audio",
            rtpParameters: rtp,
            appData: {},
          });
          if (answer.error) throw new Error(String(answer.error));
          return {
            producer: ++this.next,
            id: (answer.result as { id: string }).id,
            rtpParameters: rtp,
          };
        }
        case "media_producer_parameters":
          return { encodings: [{ active: true, priority: "low" }] };
        case "media_consume":
          await this.connect(args.transport as number);
          return {
            consumer: ++this.next,
            id: (args.params as { id: string }).id,
          };
        case "media_source_microphone":
        case "media_source_screen":
          return ++this.next;
        case "media_source_state":
          return this.screenStates.shift() ?? { state: "live" };
        case "media_audio_devices":
          return this.devices;
        case "media_audio_configure": {
          const options = args.options as { input?: string };
          if (
            options.input !== undefined &&
            !this.devices.inputs.some((d) => d.id === options.input)
          )
            throw new Error(`unknown device ${options.input}`);
          return null;
        }
        default:
          return null;
      }
    };
    return (await answer()) as T;
  }
}

let core: FakeCore;
beforeEach(() => {
  core = new FakeCore();
  setNativeBridgeForTests(core);
  resetNativeOutputForTests();
});
afterEach(() => {
  setNativeBridgeForTests(undefined);
  vi.restoreAllMocks();
});

function connection() {
  const requests: Array<{ method: MediaMethod; data: unknown }> = [];
  const consumers: ReceivedSource[] = [];
  let transports = 0;
  const request = vi.fn(async (method: MediaMethod, data: unknown) => {
    requests.push({ method, data });
    if (method === "transport")
      return {
        id: `server-transport-${++transports}`,
        iceParameters: { usernameFragment: "u", password: "p" },
        iceCandidates: [],
        dtlsParameters: { fingerprints: [] },
      };
    if (method === "produce") return { producerId: "server-producer-1" };
    return {};
  });
  const onError = vi.fn();
  const media = createMediaConnection({
    role: "voice",
    generation: GENERATION,
    iceServers: [{ urls: ["turn:turn.example.org"] }],
    request: request as MediaRequest,
    codecOptions: () => ({ opusDtx: true }),
    onConsumer: (source) => consumers.push(source),
    onConsumerClosed: vi.fn(),
    onError,
    onTransportState: vi.fn(),
  });
  return { media, requests, consumers, onError };
}

async function microphone() {
  const settings = useMediaSettings.getState();
  return captureMicrophone(async () => {
    throw new Error("the browser path must not run");
  }, settings);
}

describe("desktop app media", () => {
  it("creates browser objects outside the desktop app", () => {
    setNativeBridgeForTests(null);
    expect(createStream()).toBeNull(); // no MediaStream in node
    expect(createAudioOutput()).toBeNull();
    setNativeBridgeForTests(core);
    expect(createStream()).toBeInstanceOf(NativeStream);
    expect(createAudioOutput()).toBeInstanceOf(NativeAudioOutput);
  });

  it("publishes the native microphone through the server's signaling", async () => {
    const { media, requests, onError } = connection();
    await media.start(capabilities);
    expect(core.calledWith("media_device_load")).toEqual([{ capabilities }]);
    expect(requests.map((r) => r.method)).toEqual([
      "capabilities",
      "transport",
    ]);

    const { processor } = await microphone();
    const track = processor.stream.getAudioTracks()[0];
    expect(isNativeTrack(track)).toBe(true);
    const sender = await media.publish({
      kind: "a",
      track,
      streamId: processor.stream.id,
      epoch: EPOCH,
    });
    expect(onError).not.toHaveBeenCalled();
    expect(sender.producerId).toBe("server-producer-1");
    expect(requests.map((r) => r.method)).toEqual([
      "capabilities",
      "transport",
      "transport",
      "connect",
      "produce",
    ]);
    const produce = requests.find((r) => r.method === "produce")!.data;
    expect(produce).toMatchObject({ k: "a", rtp, epoch: EPOCH });
    const [created] = core
      .calledWith("media_transport_create")
      .filter((args) => args.direction === "send");
    expect(created.options).toMatchObject({
      id: "server-transport-2",
      iceServers: [{ urls: ["turn:turn.example.org"] }],
    });
    expect(core.calledWith("media_produce")[0]).toMatchObject({
      options: {
        codec: "audio/opus",
        codecOptions: { opusDtx: true },
      },
    });

    // RTCRtpSender parameters stay synchronous for the session.
    expect(sender.getParameters()).toEqual({
      encodings: [{ active: true, priority: "low" }],
    });
    await sender.setParameters({
      encodings: [{ maxBitrate: 64_000, priority: "high" }],
    });
    expect(core.calledWith("media_producer_set_parameters")).toEqual([
      {
        producer: expect.any(Number),
        parameters: { encodings: [{ maxBitrate: 64_000, priority: "high" }] },
      },
    ]);

    // Mute: the session disables the track and pauses the producer.
    track.enabled = false;
    await media.setSourcePaused("a", true);
    expect(core.calledWith("media_source_set_enabled")).toEqual([
      { source: expect.any(Number), enabled: false },
    ]);
    expect(core.calledWith("media_producer_pause").at(-1)).toMatchObject({
      paused: true,
    });

    await media.closeSource("a");
    expect(core.calledWith("media_producer_close")).toHaveLength(1);
    media.close();
    await tick();
    expect(core.calledWith("media_transport_close")).toHaveLength(2);
  });

  it("plays consumers natively through audio outputs", async () => {
    const { media, requests, consumers } = connection();
    await media.start(capabilities);
    media.handleEvent({
      op: "consumer",
      consumerId: "consumer-1",
      producerId: "remote-producer",
      owner: OWNER,
      k: "a",
      epoch: EPOCH,
      generation: GENERATION,
      kind: "audio",
      rtpParameters: rtp,
      paused: false,
    });
    await vi.waitFor(() => expect(consumers).toHaveLength(1));
    await vi.waitFor(() =>
      expect(requests.map((r) => r.method)).toContain("consumerReady"),
    );
    const [source] = consumers;
    expect(core.calledWith("media_consume")[0]).toMatchObject({
      params: {
        id: "consumer-1",
        producerId: "remote-producer",
        kind: "audio",
      },
    });
    const consumer = (source.track as unknown as NativeTrack).handle.consumer;
    expect(consumer).toEqual(expect.any(Number));

    const output = new NativeAudioOutput();
    output.srcObject = source.stream;
    output.volume = 0.5;
    await output.play();
    expect(core.calledWith("media_consumer_set_volume").at(-1)).toEqual({
      consumer,
      volume: 0.5,
    });
    output.muted = true;
    expect(core.calledWith("media_consumer_set_volume").at(-1)).toEqual({
      consumer,
      volume: 0,
    });
    output.muted = false;
    output.srcObject = null;
    expect(core.calledWith("media_consumer_set_volume").at(-1)).toEqual({
      consumer,
      volume: 0,
    });

    // A track joining a playing mixed stream starts audible.
    const mix = new NativeStream();
    output.srcObject = mix as unknown as MediaStream;
    mix.addTrack(source.track);
    expect(core.calledWith("media_consumer_set_volume").at(-1)).toEqual({
      consumer,
      volume: 0.5,
    });

    await output.setSinkId("Speakers");
    await output.setSinkId("Speakers");
    expect(core.calledWith("media_audio_configure")).toEqual([
      { options: { output: "Speakers" } },
    ]);
    media.close();
  });

  it("captures the microphone natively with the session's settings", async () => {
    useMediaSettings.setState({
      processingMode: "browser",
      noiseSuppression: true,
      autoGainControl: false,
      echoCancellation: true,
      inputGain: 1.5,
      audioInputId: "gone-device",
    });
    const { raw, processor } = await microphone();
    // Unknown device: the system default, like the browser's `ideal`.
    expect(core.calledWith("media_audio_configure")).toEqual([
      { options: { input: "gone-device" } },
      { options: { input: "" } },
    ]);
    expect(core.calledWith("media_source_microphone")).toEqual([
      {
        options: {
          processingMode: "browser",
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: false,
          inputGain: 1.5,
        },
      },
    ]);
    expect(processor.stream).not.toBe(raw);
    expect(processor.info).toMatchObject({
      actual: "browser",
      noiseSuppression: true,
      autoGainControl: false,
    });
    expect(processor.info.message).toContain("Standardmikrofon");
    // Stopping the placeholder raw stream releases nothing.
    raw.getTracks().forEach((track) => track.stop());
    expect(core.calledWith("media_source_close")).toEqual([]);
    processor.setGain(0.5);
    expect(core.calledWith("media_audio_configure").at(-1)).toEqual({
      options: { inputGain: 0.5 },
    });
    processor.dispose();
    expect(core.calledWith("media_source_close")).toHaveLength(1);
    expect(processor.usable()).toBe(false);
  });

  it("lists the native engine's devices", async () => {
    expect(await listMediaDevices()).toEqual({
      audioinput: [{ id: "Headset", label: "Headset" }],
      audiooutput: [{ id: "Speakers", label: "Speakers" }],
      videoinput: [],
    });
  });

  it("shares a screen picked in the desktop's dialog", async () => {
    core.screenStates = [
      { state: "pending" },
      { state: "live", width: 1920, height: 1080 },
    ];
    const stream = await nativeGetDisplayMedia({
      video: { frameRate: { ideal: 60, max: 60 } },
    });
    expect(core.calledWith("media_source_screen")).toEqual([
      {
        options: { type: "any", fps: 60, cursor: true, contentHint: "detail" },
      },
    ]);
    const [track] = stream.getVideoTracks();
    expect(track.getSettings()).toMatchObject({ width: 1920, height: 1080 });
    const ended = vi.fn();
    track.addEventListener("ended", ended);
    core.screenStates = [{ state: "ended" }];
    await vi.waitFor(() => expect(ended).toHaveBeenCalled(), {
      timeout: 3_000,
    });
    expect(track.readyState).toBe("ended");
  });

  it("reports a cancelled picker like the browser", async () => {
    core.screenStates = [{ state: "cancelled" }];
    await expect(nativeGetDisplayMedia({ video: true })).rejects.toMatchObject({
      name: "NotAllowedError",
    });
    await tick();
    expect(core.calledWith("media_source_close")).toHaveLength(1);
  });
});
