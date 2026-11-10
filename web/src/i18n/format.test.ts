import { afterEach, describe, expect, it } from "vitest";

import {
  compareNames,
  formatDate,
  formatNumber,
  formatRelative,
  formatTime,
} from "./format.ts";
import { setLocale } from "./locale.ts";

afterEach(() => setLocale("de", { persist: false }));

const date = new Date(Date.UTC(2026, 10, 10, 15, 4));

describe("format", () => {
  it("formats in German by default and in British English after a switch", () => {
    const options = { timeZone: "UTC" } as const;
    expect(formatNumber(1234.5)).toBe("1.234,5");
    expect(formatDate(date, { ...options, dateStyle: "long" })).toBe(
      "10. November 2026",
    );
    expect(formatTime(date, { ...options, timeStyle: "short" })).toBe("15:04");
    expect(formatRelative(-1, "day", { numeric: "auto" })).toBe("gestern");
    setLocale("en", { persist: false });
    expect(formatNumber(1234.5)).toBe("1,234.5");
    expect(formatDate(date, { ...options, dateStyle: "long" })).toBe(
      "10 November 2026",
    );
    expect(formatTime(date, { ...options, timeStyle: "short" })).toBe("15:04");
    expect(formatRelative(-1, "day", { numeric: "auto" })).toBe("yesterday");
  });

  it("sorts names as the language does", () => {
    expect(["Zoe", "Ömer", "Anna"].sort(compareNames)).toEqual([
      "Anna",
      "Ömer",
      "Zoe",
    ]);
  });
});
