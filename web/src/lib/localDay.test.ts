import { describe, expect, it } from "vitest";

import { localDay, msUntilNextDay } from "./localDay.ts";

describe("local day", () => {
  it("counts down to the next local midnight", () => {
    expect(msUntilNextDay(new Date(2026, 9, 22, 23, 59, 30))).toBe(30_000);
    expect(msUntilNextDay(new Date(2026, 9, 22, 0, 0, 0))).toBe(
      new Date(2026, 9, 23).getTime() - new Date(2026, 9, 22).getTime(),
    );
  });

  it("changes exactly when the date does", () => {
    const evening = new Date(2026, 9, 22, 23, 59, 59);
    const night = new Date(2026, 9, 23, 0, 0, 1);
    expect(localDay(evening)).not.toBe(localDay(night));
    expect(localDay(new Date(2026, 9, 22, 8))).toBe(localDay(evening));
  });
});
