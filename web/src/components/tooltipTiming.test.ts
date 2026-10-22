import { describe, expect, it } from "vitest";

import { TIP_DELAY_MS, tipDelay } from "./tooltipTiming.ts";

describe("tooltip timing", () => {
  const cold = { showing: false, warm: false, keyboard: false };

  it("waits on a first hover", () => {
    expect(tipDelay(cold)).toBe(TIP_DELAY_MS);
  });

  it("moves at once between neighbours and shortly after one went", () => {
    expect(tipDelay({ ...cold, showing: true })).toBe(0);
    expect(tipDelay({ ...cold, warm: true })).toBe(0);
  });

  it("shows at once for keyboard focus", () => {
    expect(tipDelay({ ...cold, keyboard: true })).toBe(0);
  });
});
