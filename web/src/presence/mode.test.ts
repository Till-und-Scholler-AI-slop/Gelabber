import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../api/client.ts", () => ({ api: vi.fn() }));

import { api } from "../api/client.ts";
import { setLocale } from "../i18n/locale.ts";
import {
  isDoNotDisturb,
  isInvisible,
  modeAt,
  modeLabel,
  normalizeMode,
  ownShown,
  parseMode,
  resetPresenceMode,
  setPresenceMode,
  usePresenceMode,
} from "./mode.ts";

const request = vi.mocked(api);
const NOW = Date.parse("2026-11-15T15:00:00Z");

afterEach(() => {
  request.mockReset();
  resetPresenceMode();
});

describe("manual presence", () => {
  it("reads the server's spellings, an unknown mode as automatic", () => {
    expect(parseMode("dnd")).toBe("dnd");
    expect(parseMode("busy")).toBe("auto");
    expect(parseMode(undefined)).toBe("auto");
    // The REST answer and the account frame name the fields differently.
    expect(
      normalizeMode({ mode: "dnd", until: "2026-11-15T18:30:00Z" }, NOW),
    ).toEqual({ mode: "dnd", until: "2026-11-15T18:30:00Z" });
    expect(normalizeMode({ m: "invisible" }, NOW)).toEqual({
      mode: "invisible",
      until: null,
    });
  });

  it("is automatic once its end has passed", () => {
    const state = { mode: "dnd" as const, until: "2026-11-15T15:30:00Z" };
    expect(modeAt(state, NOW)).toBe("dnd");
    expect(modeAt(state, NOW + 30 * 60_000)).toBe("auto");
    expect(normalizeMode(state, NOW + 31 * 60_000)).toEqual({
      mode: "auto",
      until: null,
    });
    // Automatic never keeps an end.
    expect(normalizeMode({ mode: "auto", until: state.until }, NOW)).toEqual({
      mode: "auto",
      until: null,
    });
  });

  it("shows your chosen mode on your own avatar, invisible as such", () => {
    expect(ownShown("auto", "o")).toBe("o");
    expect(ownShown("auto", "i")).toBe("i");
    // No server reported you yet: you are using the app.
    expect(ownShown("auto", "x")).toBe("o");
    expect(ownShown("idle", "o")).toBe("i");
    expect(ownShown("dnd", "o")).toBe("d");
    // Others see you offline; you see that you are hidden.
    expect(ownShown("invisible", "x")).toBe("v");
  });

  it("names the end, with the day when it is not today", () => {
    setLocale("de");
    const today = { mode: "dnd" as const, until: "2026-11-15T17:30:00Z" };
    expect(modeLabel(today, NOW)).toMatch(/^Nicht stören bis \d\d:\d\d$/);
    const tomorrow = { mode: "idle" as const, until: "2026-11-16T14:00:00Z" };
    expect(modeLabel(tomorrow, NOW)).toMatch(/^Abwesend bis \S+, \d\d:\d\d$/);
    expect(modeLabel({ mode: "invisible", until: null }, NOW)).toBe(
      "Unsichtbar",
    );
  });

  it("follows a choice at once and keeps what the server saved", async () => {
    request.mockResolvedValueOnce({
      mode: "dnd",
      until: "2099-01-01T08:00:00Z",
    });
    const saving = setPresenceMode("dnd", "8h");
    expect(usePresenceMode.getState().mode).toBe("dnd");
    expect(usePresenceMode.getState().until).not.toBeNull();
    expect(isDoNotDisturb()).toBe(true);
    await expect(saving).resolves.toEqual({
      mode: "dnd",
      until: "2099-01-01T08:00:00Z",
    });
    expect(request).toHaveBeenCalledWith("/me/presence", {
      method: "PUT",
      body: { mode: "dnd", duration: "8h" },
    });
    expect(usePresenceMode.getState().until).toBe("2099-01-01T08:00:00Z");
  });

  it("goes back to what was there when saving fails", async () => {
    request.mockResolvedValueOnce({ mode: "invisible", until: null });
    await setPresenceMode("invisible");
    expect(isInvisible()).toBe(true);
    request.mockRejectedValueOnce(new Error("offline"));
    await expect(setPresenceMode("auto")).rejects.toThrow("offline");
    expect(usePresenceMode.getState().mode).toBe("invisible");
    expect(request).toHaveBeenLastCalledWith("/me/presence", {
      method: "PUT",
      body: { mode: "auto" },
    });
  });

  it("lets only the newest choice speak", async () => {
    let first!: (value: unknown) => void;
    request.mockReturnValueOnce(
      new Promise((resolve) => {
        first = resolve;
      }),
    );
    request.mockResolvedValueOnce({ mode: "idle", until: null });
    const older = setPresenceMode("dnd");
    await setPresenceMode("idle");
    first({ mode: "dnd", until: null });
    await older;
    expect(usePresenceMode.getState().mode).toBe("idle");
  });
});
