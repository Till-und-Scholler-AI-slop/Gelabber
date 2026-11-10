import { afterEach, describe, expect, it, vi } from "vitest";

import type { MediaPeer } from "./mediaPeer.ts";
import type { MediaConnection, MediaSender } from "./mediasoupConnection.ts";
import type { RtpSenderParameters } from "./mediasoupConnection.ts";
import {
  encodingBitrates,
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
