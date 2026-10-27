import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  RtpCapabilities,
  RtpCodecCapability,
} from "mediasoup-client/types";
import {
  chooseVideoCodec,
  decodesAv1,
  hardwareVideo,
  publishableH264,
  receivableCapabilities,
  resetHardwareVideoForTests,
} from "./videoCodec.ts";
import { setNativeBridgeForTests, type NativeBridge } from "./native/bridge.ts";

const codec = (
  mimeType: string,
  parameters?: Record<string, unknown>,
): RtpCodecCapability => ({
  kind: mimeType.startsWith("audio") ? "audio" : "video",
  mimeType,
  preferredPayloadType: 96,
  clockRate: 90000,
  ...(parameters ? { parameters } : {}),
});
const vp8 = codec("video/VP8");
const vp9 = codec("video/VP9", { "profile-id": 0 });
const av1 = codec("video/AV1");
const baseline = codec("video/H264", {
  "packetization-mode": 1,
  "profile-level-id": "42001f",
});
const constrained = codec("video/H264", {
  "packetization-mode": 1,
  "profile-level-id": "42e01f",
});
const modeZero = codec("video/H264", {
  "packetization-mode": 0,
  "profile-level-id": "42e01f",
});
const high = codec("video/H264", {
  "packetization-mode": 1,
  "profile-level-id": "640032",
});
const device = [baseline, vp8, modeZero, constrained, high, vp9, av1];
const none = { av1: false, h264: false };
const h264Only = { av1: false, h264: true };
const av1Only = { av1: true, h264: false };
const both = { av1: true, h264: true };

afterEach(() => {
  resetHardwareVideoForTests();
  setNativeBridgeForTests(undefined);
  vi.unstubAllGlobals();
});

describe("video codec choice", () => {
  it("sends H.264 only as Constrained Baseline in packetization mode 1", () => {
    expect(publishableH264(constrained)).toBe(true);
    expect(
      publishableH264(
        codec("video/H264", {
          "packetization-mode": 1,
          "profile-level-id": "42C01F",
        }),
      ),
    ).toBe(true);
    for (const other of [baseline, modeZero, high, vp8])
      expect(publishableH264(other)).toBe(false);
  });

  it("prefers H.264 where it is encoded in hardware and everyone decodes it", () => {
    const allowed = ["video/vp8", "video/vp9", "video/h264", "video/av1"];
    expect(chooseVideoCodec(device, allowed, h264Only)).toBe(constrained);
    expect(chooseVideoCodec(device, allowed, none)).toBe(vp8);
    expect(chooseVideoCodec(device, ["video/vp8"], h264Only)).toBe(vp8);
    // A device without a publishable H.264 falls back to VP8.
    expect(chooseVideoCodec([baseline, vp8], allowed, h264Only)).toBe(vp8);
    expect(chooseVideoCodec(device, ["video/av1"], none)).toBe(av1);
  });

  it("prefers AV1 where it is encoded in hardware, then H.264, then VP8", () => {
    const allowed = ["video/vp8", "video/vp9", "video/h264", "video/av1"];
    expect(chooseVideoCodec(device, allowed, both)).toBe(av1);
    expect(chooseVideoCodec(device, allowed, av1Only)).toBe(av1);
    // A receiver without AV1 keeps the room on H.264 or VP8.
    const noAv1 = ["video/vp8", "video/vp9", "video/h264"];
    expect(chooseVideoCodec(device, noAv1, both)).toBe(constrained);
    expect(chooseVideoCodec(device, noAv1, av1Only)).toBe(vp8);
    // AV1 on the CPU never comes before VP8, nor before H.264 in hardware.
    expect(chooseVideoCodec(device, allowed, none)).toBe(vp8);
    expect(chooseVideoCodec(device, allowed, h264Only)).toBe(constrained);
    // A device that cannot send AV1 after all.
    expect(chooseVideoCodec([constrained, vp8], allowed, both)).toBe(
      constrained,
    );
  });

  it("keeps the old order when the server does not name codecs", () => {
    expect(chooseVideoCodec(device, null, both)).toBe(vp8);
    expect(chooseVideoCodec([baseline, av1], null, both)).toBe(baseline);
    expect(chooseVideoCodec([vp9], [], none)).toBe(vp9);
    expect(chooseVideoCodec([], null, none)).toBeUndefined();
  });
});

describe("hardware video", () => {
  const answer =
    (efficient: Record<string, boolean>) =>
    async ({ video }: { video: { contentType: string } }) => ({
      supported: true,
      smooth: true,
      powerEfficient: efficient[video.contentType] ?? false,
    });

  it("is what the browser calls power efficient, asked once", async () => {
    const encodingInfo = vi.fn(
      answer({ "video/H264": true, "video/AV1": false }),
    );
    vi.stubGlobal("navigator", { mediaCapabilities: { encodingInfo } });
    expect(await hardwareVideo()).toEqual(h264Only);
    expect(await hardwareVideo()).toEqual(h264Only);
    expect(encodingInfo).toHaveBeenCalledTimes(2);
    expect(encodingInfo.mock.calls.map(([query]) => query)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: "webrtc",
          video: expect.objectContaining({ contentType: "video/H264" }),
        }),
        expect.objectContaining({
          type: "webrtc",
          video: expect.objectContaining({ contentType: "video/AV1" }),
        }),
      ]),
    );
    resetHardwareVideoForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: answer({ "video/H264": true, "video/AV1": true }),
      },
    });
    expect(await hardwareVideo()).toEqual(both);
  });

  it("is software when the browser cannot say or says no", async () => {
    vi.stubGlobal("navigator", {});
    expect(await hardwareVideo()).toEqual(none);
    resetHardwareVideoForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: { encodingInfo: answer({}) },
    });
    expect(await hardwareVideo()).toEqual(none);
    resetHardwareVideoForTests();
    // A browser that knows H.264 but not the AV1 question.
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: async ({ video }: { video: { contentType: string } }) => {
          if (video.contentType === "video/AV1")
            throw new TypeError("codec unknown");
          return { supported: true, smooth: true, powerEfficient: true };
        },
      },
    });
    expect(await hardwareVideo()).toEqual(h264Only);
    resetHardwareVideoForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: async () => {
          throw new TypeError("webrtc type unknown");
        },
      },
    });
    expect(await hardwareVideo()).toEqual(none);
  });

  it("is what the desktop app's features say", async () => {
    const encodingInfo = vi.fn(answer({ "video/H264": true }));
    vi.stubGlobal("navigator", { mediaCapabilities: { encodingInfo } });
    setNativeBridgeForTests({
      invoke: async (command: string) =>
        command === "media_info"
          ? { features: ["screen", "av1-hardware"] }
          : Promise.reject(new Error(command)),
      channel: async () => null,
    } as unknown as NativeBridge);
    expect(await hardwareVideo()).toEqual(av1Only);
    expect(await decodesAv1()).toBe(false);
    expect(encodingInfo).not.toHaveBeenCalled();
    resetHardwareVideoForTests();
    setNativeBridgeForTests({
      invoke: async (command: string) =>
        command === "media_info"
          ? { features: ["av1-hardware-decode"] }
          : Promise.reject(new Error(command)),
      channel: async () => null,
    } as unknown as NativeBridge);
    expect(await decodesAv1()).toBe(true);
    expect(await hardwareVideo()).toEqual(none);
  });

  it("receives AV1 where the browser decodes 4K smoothly, in hardware or not", async () => {
    const decodingInfo = vi.fn<(query: unknown) => Promise<object>>(async () => ({
      supported: true,
      smooth: true,
      powerEfficient: false,
    }));
    vi.stubGlobal("navigator", {
      mediaCapabilities: { encodingInfo: answer({}), decodingInfo },
    });
    expect(await decodesAv1()).toBe(true);
    expect(await hardwareVideo()).toEqual(none);
    expect(decodingInfo).toHaveBeenCalledOnce();
    expect(decodingInfo.mock.calls[0][0]).toEqual({
      type: "webrtc",
      video: expect.objectContaining({
        contentType: "video/AV1",
        width: 3840,
        height: 2160,
      }),
    });
    resetHardwareVideoForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: answer({}),
        decodingInfo: async () => ({
          supported: true,
          smooth: false,
          powerEfficient: true,
        }),
      },
    });
    expect(await decodesAv1()).toBe(false);
    resetHardwareVideoForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: { encodingInfo: answer({ "video/AV1": true }) },
    });
    expect(await decodesAv1()).toBe(false);
  });

  describe("with a slow desktop app", () => {
    const slowApp = (delayMs: number, features: string[]) => {
      const invoke = vi.fn(
        (command: string) =>
          new Promise((resolve, reject) => {
            if (command !== "media_info") return reject(new Error(command));
            setTimeout(() => resolve({ features }), delayMs);
          }),
      );
      setNativeBridgeForTests({
        invoke,
        channel: async () => null,
      } as unknown as NativeBridge);
      return invoke;
    };

    afterEach(() => {
      vi.useRealTimers();
    });

    it("waits for it and does not keep a timed-out answer", async () => {
      vi.useFakeTimers();
      slowApp(3000, ["h264-hardware"]);
      const early = hardwareVideo();
      await vi.advanceTimersByTimeAsync(3000);
      expect((await early).h264).toBe(true);

      resetHardwareVideoForTests();
      slowApp(11_000, ["h264-hardware"]);
      const first = hardwareVideo();
      await vi.advanceTimersByTimeAsync(10_000);
      expect(await first).toEqual(none);
      // The app answers late; the timed-out answer was not kept.
      await vi.advanceTimersByTimeAsync(1000);
      expect((await hardwareVideo()).h264).toBe(true);
    });
  });
});

describe("receivable capabilities", () => {
  const rtx = (apt: number): RtpCodecCapability => ({
    ...codec("video/rtx", { apt }),
    preferredPayloadType: apt + 1,
  });
  const av1At45 = { ...av1, preferredPayloadType: 45 };
  const vp8At96 = { ...vp8, preferredPayloadType: 96 };
  const device: RtpCapabilities = {
    codecs: [vp8At96, rtx(96), av1At45, rtx(45)],
    headerExtensions: [],
  };

  it("leaves out AV1 and its RTX without a hardware decoder", () => {
    expect(receivableCapabilities(device, false)).toEqual({
      codecs: [vp8At96, rtx(96)],
      headerExtensions: [],
    });
  });

  it("keeps everything with one", () => {
    expect(receivableCapabilities(device, true)).toBe(device);
  });
});
