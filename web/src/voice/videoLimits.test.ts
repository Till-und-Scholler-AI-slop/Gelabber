import { describe, expect, it } from "vitest";

import { encodingBitrates, videoBitrateCeiling } from "./videoLimits.ts";

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
