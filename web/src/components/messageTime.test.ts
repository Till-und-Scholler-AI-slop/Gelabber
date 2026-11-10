import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import { dayLabel, formatFull, formatWhen } from "./messageTime.ts";

afterEach(() => setLocale("de", { persist: false }));

describe("formatWhen", () => {
  const now = new Date(2026, 10, 8, 18, 0);
  const at = (y: number, m: number, d: number) =>
    new Date(y, m, d, 16, 24).toISOString();

  it("names the day only off today and the year only off this year", () => {
    expect(formatWhen(at(2026, 10, 8), now)).toBe("16:24");
    expect(formatWhen(at(2026, 10, 5), now)).toBe("05.11., 16:24");
    expect(formatWhen(at(2025, 10, 8), now)).toBe("08.11.2025, 16:24");
  });

  it("writes dates the British way in English", () => {
    setLocale("en", { persist: false });
    expect(formatWhen(at(2026, 10, 8), now)).toBe("16:24");
    expect(formatWhen(at(2026, 10, 5), now)).toBe("05/11, 16:24");
    expect(formatWhen(at(2025, 10, 8), now)).toBe("08/11/2025, 16:24");
  });
});

describe("dayLabel and formatFull", () => {
  it("follow the language", () => {
    const today = new Date().toISOString();
    const day = new Date(2025, 9, 21, 14, 2).toISOString();
    expect(dayLabel(today)).toBe("Heute");
    expect(dayLabel(day)).toBe("Dienstag, 21. Oktober 2025");
    expect(formatFull(day)).toBe("Dienstag, 21. Oktober 2025 um 14:02");
    setLocale("en", { persist: false });
    expect(dayLabel(today)).toBe("Today");
    // ICU versions differ on the comma after the weekday.
    expect(dayLabel(day)).toMatch(/^Tuesday,? 21 October 2025$/);
    expect(formatFull(day)).toMatch(/^Tuesday,? 21 October 2025 at 14:02$/);
  });
});
