import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import { deviceLabel, lastActiveLabel } from "./devices.ts";

afterEach(() => setLocale("de", { persist: false }));

describe("deviceLabel", () => {
  it("names the app and browsers with their system", () => {
    expect(deviceLabel({ client: "desktop", os: "linux" })).toBe(
      "Gelabber für Linux",
    );
    expect(deviceLabel({ client: "firefox", os: "windows" })).toBe(
      "Firefox unter Windows",
    );
    expect(deviceLabel({ client: "chrome", os: "android" })).toBe(
      "Chrome unter Android",
    );
    expect(deviceLabel({ client: "edge", os: "windows" })).toBe(
      "Edge unter Windows",
    );
    expect(deviceLabel({ client: "safari", os: "ios" })).toBe(
      "Safari unter iOS",
    );
    expect(deviceLabel({ client: "browser", os: "macos" })).toBe(
      "Browser unter macOS",
    );
  });

  it("leaves out a system it does not know", () => {
    expect(deviceLabel({ client: "desktop", os: "other" })).toBe("Gelabber");
    expect(deviceLabel({ client: "firefox", os: "other" })).toBe("Firefox");
    expect(deviceLabel({ client: "browser", os: "other" })).toBe("Browser");
  });

  it("calls a client it cannot tell an unknown device", () => {
    expect(deviceLabel({ client: "unknown", os: "linux" })).toBe(
      "Unbekanntes Gerät",
    );
    expect(deviceLabel({ client: "unknown", os: "other" })).toBe(
      "Unbekanntes Gerät",
    );
  });

  it("speaks English with the interface", () => {
    setLocale("en", { persist: false });
    expect(deviceLabel({ client: "desktop", os: "windows" })).toBe(
      "Gelabber for Windows",
    );
    expect(deviceLabel({ client: "firefox", os: "linux" })).toBe(
      "Firefox on Linux",
    );
    expect(deviceLabel({ client: "browser", os: "android" })).toBe(
      "Browser on Android",
    );
    expect(deviceLabel({ client: "unknown", os: "other" })).toBe(
      "Unknown device",
    );
  });
});

describe("lastActiveLabel", () => {
  // Local times, so the calendar day is the reader's.
  const now = new Date(2026, 10, 11, 9, 30);
  const at = (d: number, h = 12) => new Date(2026, 10, d, h).toISOString();
  const other = (last_used_at: string) =>
    lastActiveLabel({ last_used_at, current: false }, now);

  it("says just now for this device, whatever the time", () => {
    expect(lastActiveLabel({ last_used_at: at(1), current: true }, now)).toBe(
      "Zuletzt aktiv: gerade eben",
    );
  });

  it("names the local calendar day of another device", () => {
    expect(other(new Date(2026, 10, 11, 0, 5).toISOString())).toBe(
      "Zuletzt aktiv: heute",
    );
    // Late yesterday is yesterday, though less than a day ago.
    expect(other(new Date(2026, 10, 10, 23, 50).toISOString())).toBe(
      "Zuletzt aktiv: gestern",
    );
    expect(other(at(9, 1))).toBe("Zuletzt aktiv: vor 2 Tagen");
    expect(other(new Date(2026, 9, 30, 8).toISOString())).toBe(
      "Zuletzt aktiv: vor 12 Tagen",
    );
  });

  it("reads a time ahead of this clock as today", () => {
    expect(other(new Date(2026, 10, 11, 10).toISOString())).toBe(
      "Zuletzt aktiv: heute",
    );
  });

  it("speaks English with the interface", () => {
    setLocale("en", { persist: false });
    expect(lastActiveLabel({ last_used_at: at(1), current: true }, now)).toBe(
      "Last active: just now",
    );
    expect(other(at(10))).toBe("Last active: yesterday");
    expect(other(at(1))).toBe("Last active: 10 days ago");
  });
});
