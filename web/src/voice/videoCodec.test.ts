import { afterEach, describe, expect, it, vi } from "vitest";
import type { RtpCodecCapability } from "mediasoup-client/types";
import {
  chooseVideoCodec,
  hardwareH264,
  publishableH264,
  resetHardwareH264ForTests,
} from "./videoCodec.ts";

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

afterEach(() => {
  resetHardwareH264ForTests();
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
    expect(chooseVideoCodec(device, allowed, true)).toBe(constrained);
    expect(chooseVideoCodec(device, allowed, false)).toBe(vp8);
    expect(chooseVideoCodec(device, ["video/vp8"], true)).toBe(vp8);
    // A device without a publishable H.264 falls back to VP8.
    expect(chooseVideoCodec([baseline, vp8], allowed, true)).toBe(vp8);
    expect(chooseVideoCodec(device, ["video/av1"], false)).toBe(av1);
  });

  it("keeps the old order when the server does not name codecs", () => {
    expect(chooseVideoCodec(device, null, true)).toBe(vp8);
    expect(chooseVideoCodec([baseline, av1], null, true)).toBe(baseline);
    expect(chooseVideoCodec([vp9], [], false)).toBe(vp9);
    expect(chooseVideoCodec([], null, false)).toBeUndefined();
  });
});

describe("hardware H.264", () => {
  it("is what the browser calls power efficient, asked once", async () => {
    const encodingInfo = vi.fn(async () => ({
      supported: true,
      smooth: true,
      powerEfficient: true,
    }));
    vi.stubGlobal("navigator", { mediaCapabilities: { encodingInfo } });
    expect(await hardwareH264()).toBe(true);
    expect(await hardwareH264()).toBe(true);
    expect(encodingInfo).toHaveBeenCalledTimes(1);
    expect(encodingInfo.mock.calls[0]).toMatchObject([
      { type: "webrtc", video: { contentType: "video/H264" } },
    ]);
  });

  it("is software when the browser cannot say or says no", async () => {
    vi.stubGlobal("navigator", {});
    expect(await hardwareH264()).toBe(false);
    resetHardwareH264ForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: async () => ({
          supported: true,
          smooth: true,
          powerEfficient: false,
        }),
      },
    });
    expect(await hardwareH264()).toBe(false);
    resetHardwareH264ForTests();
    vi.stubGlobal("navigator", {
      mediaCapabilities: {
        encodingInfo: async () => {
          throw new TypeError("webrtc type unknown");
        },
      },
    });
    expect(await hardwareH264()).toBe(false);
  });
});
