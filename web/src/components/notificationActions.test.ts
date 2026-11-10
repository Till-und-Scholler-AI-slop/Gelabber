import { afterEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import {
  LEVELS,
  MUTES,
  formatUntil,
  mutedLine,
} from "./notificationActions.ts";

describe("notification choices", () => {
  afterEach(() => {
    vi.useRealTimers();
    setLocale("de", { persist: false });
  });

  it("names the end of a mute in the interface's language", () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date(2026, 10, 3, 9, 0));
    const today = new Date(2026, 10, 3, 14, 30).toISOString();
    const later = new Date(2026, 10, 5, 14, 30).toISOString();

    expect(formatUntil(today)).toBe("14:30 Uhr");
    expect(formatUntil(later)).toBe("Do., 5. Nov., 14:30 Uhr");
    expect(mutedLine(null)).toBe("Stumm, bis du es aufhebst");

    setLocale("en", { persist: false });
    expect(formatUntil(today)).toBe("14:30");
    expect(formatUntil(later)).toBe("Thu 5 Nov, 14:30");
    expect(mutedLine(today)).toBe("Muted until 14:30");
  });

  it("reads the mute and level names when they are shown", () => {
    expect(MUTES.map((mute) => mute.short)).toEqual([
      "15 Min.",
      "1 Std.",
      "8 Std.",
      "24 Std.",
      "Unbegrenzt",
    ]);
    expect(MUTES.map((mute) => mute.minutes)).toEqual([
      15,
      60,
      480,
      1440,
      undefined,
    ]);
    setLocale("en", { persist: false });
    expect(MUTES[0].label).toBe("15 minutes");
    expect(LEVELS.map((row) => row.label)).toEqual([
      "All messages",
      "Only @mentions",
      "Nothing",
    ]);
  });
});
