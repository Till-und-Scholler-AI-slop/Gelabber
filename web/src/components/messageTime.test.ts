import { describe, expect, it } from "vitest";

import { formatWhen } from "./messageTime.ts";

describe("formatWhen", () => {
  const now = new Date(2026, 10, 8, 18, 0);
  const at = (y: number, m: number, d: number) =>
    new Date(y, m, d, 16, 24).toISOString();

  it("names the day only off today and the year only off this year", () => {
    expect(formatWhen(at(2026, 10, 8), now)).toBe("16:24");
    expect(formatWhen(at(2026, 10, 5), now)).toBe("05.11., 16:24");
    expect(formatWhen(at(2025, 10, 8), now)).toBe("08.11.2025, 16:24");
  });
});
