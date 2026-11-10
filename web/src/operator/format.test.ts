import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import {
  formatBytes as bytesWithNbsp,
  formatCount,
  formatWhen,
} from "./format.ts";

// Intl puts a no-break space between number and unit.
const formatBytes = (bytes: number) =>
  bytesWithNbsp(bytes).replace(/\u00a0/g, " ");

afterEach(() => setLocale("de", { persist: false }));

const now = Date.UTC(2026, 10, 10, 12, 0);
const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

describe("operator formats", () => {
  it("writes sizes as the reference does, in German", () => {
    expect(formatBytes(6_800_000_000)).toBe("6,8 GB");
    expect(formatBytes(340_000_000)).toBe("340 MB");
    expect(formatBytes(1_200_000_000)).toBe("1,2 GB");
    expect(formatBytes(80_000_000)).toBe("80 MB");
    expect(formatBytes(999_600_000)).toBe("1 GB");
    expect(formatBytes(512)).toBe("512 Byte");
    expect(formatBytes(0)).toBe("0 Byte");
  });

  it("writes sizes and counts in British English after a switch", () => {
    setLocale("en", { persist: false });
    expect(formatBytes(6_800_000_000)).toBe("6.8 GB");
    expect(formatBytes(340_000_000)).toBe("340 MB");
    expect(formatCount(12345)).toBe("12,345");
    setLocale("de", { persist: false });
    expect(formatCount(12345)).toBe("12.345");
  });

  it("says when, relative to now, in both languages", () => {
    expect(formatWhen(now - 20_000, now)).toBe("in dieser Minute");
    expect(formatWhen(now - 5 * MINUTE, now)).toBe("vor 5 Minuten");
    expect(formatWhen(now - DAY, now)).toBe("gestern");
    expect(formatWhen(new Date(now + 3 * DAY).toISOString(), now)).toBe(
      "in 3 Tagen",
    );
    setLocale("en", { persist: false });
    expect(formatWhen(now - 5 * MINUTE, now)).toBe("5 minutes ago");
    expect(formatWhen(now - 2 * 60 * MINUTE, now)).toBe("2 hours ago");
    expect(formatWhen(now - 60 * DAY, now)).toBe("2 months ago");
    expect(formatWhen(now - 400 * DAY, now)).toBe("last year");
  });
});
