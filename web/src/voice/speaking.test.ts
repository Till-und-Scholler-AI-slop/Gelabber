import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ReceivedSource } from "./mediasoupConnection.ts";

const fixture = vi.hoisted(() => ({
  microphones: {
    self: "self" as string | null,
    sender: undefined as unknown,
    received: [] as unknown[],
  },
}));
vi.mock("./session.ts", async () => {
  const { create } = await import("zustand");
  return {
    useVoice: create(() => ({
      status: "idle",
      serverId: null as string | null,
      channelId: null as string | null,
      muted: false,
      deafened: false,
    })),
    voiceMicrophones: () => fixture.microphones,
  };
});

import { setNativeBridgeForTests } from "./native/bridge.ts";
import { useVoice } from "./session.ts";
import {
  SPEAKING_HOLD_MS,
  receiverLevel,
  senderLevel,
  useSpeakingStore,
  watchSpeaking,
} from "./speaking.ts";

function receiver(
  sources: Partial<RTCRtpSynchronizationSource>[],
  stats: Record<string, unknown>[] = [],
) {
  return {
    getSynchronizationSources: () => sources as RTCRtpSynchronizationSource[],
    getStats: vi.fn(async () => new Map(stats.map((s, i) => [String(i), s]))),
  } as unknown as RTCRtpReceiver;
}

function remote(owner: string, level: () => number): ReceivedSource {
  return {
    owner,
    kind: "audio",
    k: "a",
    consumer: { closed: false, paused: false },
    rtpReceiver: {
      getSynchronizationSources: () => [
        {
          audioLevel: level(),
          timestamp: performance.timeOrigin + performance.now(),
        },
      ],
      getStats: async () => new Map(),
    },
  } as unknown as ReceivedSource;
}

const join = (patch: Record<string, unknown> = {}) =>
  useVoice.setState({
    status: "joined",
    serverId: "s",
    channelId: "c",
    ...patch,
  } as never);

describe("receiverLevel", () => {
  const now = () => ({ epoch: 1_000_000, page: 5_000 });

  it("reads the newest RTP audio level", async () => {
    const level = await receiverLevel(
      receiver([
        { audioLevel: 0.01, timestamp: 999_800 },
        { audioLevel: 0.2, timestamp: 999_900 },
      ]),
      now,
    );
    expect(level).toBe(0.2);
  });

  it("accepts timestamps on the page clock as well", async () => {
    expect(
      await receiverLevel(
        receiver([{ audioLevel: 0.3, timestamp: 4_950 }]),
        now,
      ),
    ).toBe(0.3);
  });

  it("treats a level from a source gone quiet as silence", async () => {
    expect(
      await receiverLevel(
        receiver([{ audioLevel: 0.3, timestamp: 990_000 }]),
        now,
      ),
    ).toBe(0);
  });

  it("falls back to the decoder's level without the header extension", async () => {
    const r = receiver(
      [],
      [
        { type: "transport" },
        { type: "inbound-rtp", kind: "audio", audioLevel: 0.12 },
      ],
    );
    expect(await receiverLevel(r, now)).toBe(0.12);
  });

  it("reports nothing where the browser knows no level", async () => {
    expect(
      await receiverLevel(receiver([], [{ type: "inbound-rtp" }]), now),
    ).toBe(null);
  });
});

describe("senderLevel", () => {
  it("reads the audio media-source", async () => {
    const sender = {
      getStats: async () =>
        new Map([
          ["v", { type: "media-source", kind: "video" }],
          ["a", { type: "media-source", kind: "audio", audioLevel: 0.4 }],
        ]),
    } as unknown as RTCRtpSender;
    expect(await senderLevel(sender)).toBe(0.4);
  });
});

describe("watchSpeaking", () => {
  let stop: () => void = () => {};
  beforeEach(() => {
    vi.useFakeTimers();
    setNativeBridgeForTests(null);
    fixture.microphones = { self: "self", sender: undefined, received: [] };
    useVoice.setState({
      status: "idle",
      serverId: null,
      channelId: null,
      muted: false,
      deafened: false,
    } as never);
  });
  afterEach(() => {
    stop();
    vi.useRealTimers();
    setNativeBridgeForTests(undefined);
  });

  async function poll() {
    await vi.advanceTimersByTimeAsync(130);
  }

  it("marks a loud remote microphone and keeps it through short pauses", async () => {
    let level = 0.3;
    fixture.microphones.received = [remote("u1", () => level)];
    stop = watchSpeaking();
    join();
    await poll();
    expect(useSpeakingStore.getState()).toEqual({
      call: "s/c",
      users: { u1: true },
    });
    level = 0.001;
    await poll();
    expect(useSpeakingStore.getState().users).toEqual({ u1: true });
    await vi.advanceTimersByTimeAsync(SPEAKING_HOLD_MS + 100);
    expect(useSpeakingStore.getState().users).toEqual({});
  });

  it("ignores paused consumers and quiet rooms", async () => {
    const paused = remote("u1", () => 0.5);
    (paused.consumer as { paused: boolean }).paused = true;
    fixture.microphones.received = [paused, remote("u2", () => 0.005)];
    stop = watchSpeaking();
    join();
    await poll();
    expect(useSpeakingStore.getState().users).toEqual({});
  });

  it("clears everything on leave", async () => {
    fixture.microphones.received = [remote("u1", () => 0.3)];
    stop = watchSpeaking();
    join();
    await poll();
    useVoice.setState({
      status: "idle",
      serverId: null,
      channelId: null,
    } as never);
    expect(useSpeakingStore.getState()).toEqual({ call: null, users: {} });
  });

  it("reads the desktop app's meters, never the own one while muted", async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === "media_audio_levels")
        return { processed: 30, blocks: invoke.mock.calls.length };
      if (command === "media_consumer_stats") return { audioLevel: 20 };
      throw new Error(command);
    });
    setNativeBridgeForTests({ invoke, channel: async () => null } as never);
    fixture.microphones.received = [
      {
        owner: "u1",
        kind: "audio",
        k: "a",
        consumer: {
          closed: false,
          paused: false,
          playoutLevel: async () =>
            ((await invoke("media_consumer_stats")) as { audioLevel: number })
              .audioLevel,
        },
      },
    ];
    stop = watchSpeaking();
    join();
    await vi.advanceTimersByTimeAsync(260);
    expect(useSpeakingStore.getState().users).toEqual({
      self: true,
      u1: true,
    });
    join({ muted: true });
    await vi.advanceTimersByTimeAsync(260);
    expect(useSpeakingStore.getState().users).toEqual({ u1: true });
  });
});
