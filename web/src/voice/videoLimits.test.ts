import { afterEach, describe, expect, it, vi } from "vitest";

import type { MediaPeer } from "./mediaPeer.ts";
import type { MediaConnection, MediaSender } from "./mediasoupConnection.ts";
import type { RtpSenderParameters } from "./mediasoupConnection.ts";
import { resetMediaSettingsForTests, useMediaSettings } from "./settings.ts";
import {
  degradationFor,
  encodingBitrates,
  layerBitrateWeights,
  layerFrameRates,
  SHARP_FALLBACK_FPS,
  SHARP_FALLBACK_WEIGHT,
  VideoLimits,
  videoBitrateCeiling,
} from "./videoLimits.ts";

describe("video bitrate ceilings", () => {
  it("scales with pixels and frame rate", () => {
    expect(videoBitrateCeiling(3840, 2160, 30)).toBe(24_883_200);
  });

  it("gives every layer the ceiling of its own resolution without a limit", () => {
    expect(encodingBitrates([4, 2, 1], 3840, 2160, 30, null)).toEqual([
      1_555_200, 6_220_800, 24_883_200,
    ]);
    expect(encodingBitrates([4, 1], 3840, 2160, 60, null)).toEqual([
      3_110_400, 49_766_400,
    ]);
  });

  it("splits a binding limit in proportion to the ceilings", () => {
    expect(encodingBitrates([4, 2, 1], 3840, 2160, 30, 10_000_000)).toEqual([
      476_190, 1_904_761, 7_619_047,
    ]);
  });

  it("never raises a layer above its ceiling", () => {
    expect(encodingBitrates([4, 2, 1], 3840, 2160, 30, 100_000_000)).toEqual([
      1_555_200, 6_220_800, 24_883_200,
    ]);
  });
});

describe("the upload budget on a resized capture", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("follows a captured window that grows", async () => {
    vi.useFakeTimers();
    const size = { width: 640, height: 480 };
    const track = {
      kind: "video",
      readyState: "live",
      getSettings: () => ({ ...size }),
    } as unknown as MediaStreamTrack;
    let params: RtpSenderParameters = {
      encodings: [{}],
      transactionId: "tx",
    };
    const sender = {
      sourceKind: "s",
      track,
      getParameters: () => ({
        ...params,
        encodings: params.encodings.map((e) => ({ ...e })),
      }),
      setParameters: async (next: RtpSenderParameters) => {
        params = next;
      },
    } as unknown as MediaSender;
    const pc = { senders: () => [sender] } as unknown as MediaConnection;
    const seat = { generation: 1, connection: pc } as unknown as MediaPeer;
    const limits = new VideoLimits(
      () => seat,
      () => "balanced",
    );
    await limits.enqueue(pc);
    expect(params.encodings[0]?.maxBitrate).toBe(
      videoBitrateCeiling(640, 480, 30),
    );
    size.width = 1920;
    size.height = 1080;
    await vi.advanceTimersByTimeAsync(2_500);
    expect(params.encodings[0]?.maxBitrate).toBe(
      videoBitrateCeiling(1920, 1080, 30),
    );
    limits.reset();
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("a Live that keeps text sharp (GEL-26)", () => {
  afterEach(() => {
    resetMediaSettingsForTests();
  });

  it("runs the smaller layers at 15 FPS and the full-size one at the profile's rate", () => {
    expect(layerFrameRates([4, 2, 1], 30, "sharp")).toEqual([
      SHARP_FALLBACK_FPS,
      SHARP_FALLBACK_FPS,
      30,
    ]);
    expect(layerFrameRates([4, 2, 1], 30, "smooth")).toEqual([30, 30, 30]);
    // A camera, or one stream without layers (AV1, VP9 SVC).
    expect(layerFrameRates([4, 2, 1], 30, null)).toEqual([30, 30, 30]);
    expect(layerFrameRates([1], 30, "sharp")).toEqual([30]);
  });

  it("caps each layer for its own frame rate, the smaller ones at half weight", () => {
    const weights = layerBitrateWeights([4, 2, 1], "sharp");
    expect(weights).toEqual([SHARP_FALLBACK_WEIGHT, SHARP_FALLBACK_WEIGHT, 1]);
    expect(layerBitrateWeights([4, 2, 1], "smooth")).toEqual([1, 1, 1]);
    expect(
      encodingBitrates([4, 2, 1], 1920, 1080, [15, 15, 30], null, weights),
    ).toEqual([
      Math.round(videoBitrateCeiling(480, 270, 15) * SHARP_FALLBACK_WEIGHT),
      Math.round(videoBitrateCeiling(960, 540, 15) * SHARP_FALLBACK_WEIGHT),
      videoBitrateCeiling(1920, 1080, 30),
    ]);
  });

  it("keeps the resolution when sharp and the frame rate when smooth", () => {
    expect(degradationFor("sharp")).toBe("maintain-resolution");
    expect(degradationFor("smooth")).toBe("maintain-framerate");
    expect(degradationFor(null)).toBeUndefined();
  });

  async function sent(
    sourceKind: "s" | "v",
    refuseDegradation = false,
  ): Promise<RtpSenderParameters> {
    const track = {
      kind: "video",
      readyState: "live",
      getSettings: () => ({ width: 1920, height: 1080 }),
    } as unknown as MediaStreamTrack;
    let params: RtpSenderParameters = {
      encodings: [
        { scaleResolutionDownBy: 4 },
        { scaleResolutionDownBy: 2 },
        { scaleResolutionDownBy: 1 },
      ],
      transactionId: "tx",
    };
    const sender = {
      sourceKind,
      track,
      getParameters: () => ({
        ...params,
        encodings: params.encodings.map((e) => ({ ...e })),
      }),
      setParameters: async (next: RtpSenderParameters) => {
        if (refuseDegradation && next.degradationPreference)
          throw new TypeError("unsupported");
        params = next;
      },
    } as unknown as MediaSender;
    const pc = { senders: () => [sender] } as unknown as MediaConnection;
    const seat = { generation: 1, connection: pc } as unknown as MediaPeer;
    const limits = new VideoLimits(
      () => seat,
      () => "1080p30",
    );
    await limits.enqueue(pc);
    limits.reset();
    return params;
  }

  it("sets a sharp Live's sender, and a smooth one's", async () => {
    const sharp = await sent("s");
    expect(sharp.encodings.map((e) => e.maxFramerate)).toEqual([15, 15, 30]);
    expect(sharp.degradationPreference).toBe("maintain-resolution");
    useMediaSettings.getState().patch({ screenMode: "smooth" });
    const smooth = await sent("s");
    expect(smooth.encodings.map((e) => e.maxFramerate)).toEqual([30, 30, 30]);
    expect(smooth.degradationPreference).toBe("maintain-framerate");
  });

  it("leaves a camera as it was", async () => {
    const camera = await sent("v");
    expect(camera.encodings.map((e) => e.maxFramerate)).toEqual([30, 30, 30]);
    expect(camera.degradationPreference).toBeUndefined();
  });

  it("still caps a browser that refuses the degradation preference", async () => {
    const refused = await sent("s", true);
    expect(refused.encodings.map((e) => e.maxFramerate)).toEqual([15, 15, 30]);
    expect(refused.degradationPreference).toBeUndefined();
  });
});
