import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  api: vi.fn(),
  logOut: vi.fn(),
}));
vi.mock("../api/client.ts", async (original) => ({
  ...(await original<typeof import("../api/client.ts")>()),
  api: fixture.api,
}));
vi.mock("../components/logout.ts", () => ({ logOut: fixture.logOut }));

import { ApiError } from "../api/client.ts";
import { setLocale } from "../i18n/locale.ts";
import {
  deviceLabel,
  lastActiveLabel,
  sessionsKey,
  signOutDevice,
} from "./devices.ts";

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

describe("signOutDevice", () => {
  const USER = "00000000-0000-0000-0000-000000000001";
  const OTHER = { id: "00000000-0000-0000-0000-0000000000aa", current: false };
  let client: QueryClient;
  let invalidate: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    fixture.api.mockReset();
    fixture.logOut.mockReset();
    client = new QueryClient();
    invalidate = vi.spyOn(client, "invalidateQueries");
  });

  it("ends another device through the API, then fetches the list again", async () => {
    fixture.api.mockResolvedValue(null);
    await expect(signOutDevice(OTHER, USER, client)).resolves.toBe("revoked");
    expect(fixture.api).toHaveBeenCalledWith(`/auth/sessions/${OTHER.id}`, {
      method: "DELETE",
    });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: sessionsKey(USER) });
    expect(fixture.logOut).not.toHaveBeenCalled();
  });

  it("only refreshes the list for a device that is gone already", async () => {
    fixture.api.mockRejectedValue(new ApiError("not_found", 404, "gone"));
    await expect(signOutDevice(OTHER, USER, client)).resolves.toBe("revoked");
    expect(invalidate).toHaveBeenCalledWith({ queryKey: sessionsKey(USER) });
  });

  it("reports any other failure and keeps the list", async () => {
    fixture.api.mockRejectedValue(new ApiError("internal", 500, "boom"));
    await expect(signOutDevice(OTHER, USER, client)).rejects.toThrow("boom");
    expect(invalidate).not.toHaveBeenCalled();
  });

  it("signs this device out as the user panel does", async () => {
    fixture.logOut.mockResolvedValue(true);
    await expect(
      signOutDevice({ id: OTHER.id, current: true }, USER, client),
    ).resolves.toBe("signedOutHere");
    expect(fixture.logOut).toHaveBeenCalledOnce();
    expect(fixture.api).not.toHaveBeenCalled();
  });

  it("stays when signing out here was called off", async () => {
    fixture.logOut.mockResolvedValue(false);
    await expect(
      signOutDevice({ id: OTHER.id, current: true }, USER, client),
    ).resolves.toBe("stayed");
  });
});
