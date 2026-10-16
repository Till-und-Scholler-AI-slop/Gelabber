import { afterEach, describe, expect, it, vi } from "vitest";
import {
  DEFAULT_USER_VOLUME,
  effectiveVoiceVolume,
  resetUserVolumesForTests,
  userVolumeOf,
  useUserVolumes,
} from "./userVolume.ts";

function memoryStorage(initial: Record<string, string> = {}) {
  const items = new Map(Object.entries(initial));
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}

afterEach(() => {
  resetUserVolumesForTests();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("effectiveVoiceVolume", () => {
  const user = (volume: number, muted = false) => ({ volume, muted });

  it("multiplies the output volume by the participant's volume", () => {
    expect(
      effectiveVoiceVolume({ output: 0.5, user: user(1), deafened: false }),
    ).toBe(0.5);
    expect(
      effectiveVoiceVolume({ output: 0.8, user: user(0.5), deafened: false }),
    ).toBeCloseTo(0.4);
    expect(
      effectiveVoiceVolume({ output: 1, user: user(0), deafened: false }),
    ).toBe(0);
  });

  it("goes above 100 % and stops at 200 %", () => {
    expect(
      effectiveVoiceVolume({ output: 1, user: user(1.5), deafened: false }),
    ).toBe(1.5);
    expect(
      effectiveVoiceVolume({ output: 1.5, user: user(2), deafened: false }),
    ).toBe(2);
    expect(
      effectiveVoiceVolume({ output: 9, user: user(9), deafened: false }),
    ).toBe(2);
  });

  it("is silent while locally muted or deafened, whatever the levels", () => {
    expect(
      effectiveVoiceVolume({ output: 1, user: user(2, true), deafened: false }),
    ).toBe(0);
    expect(
      effectiveVoiceVolume({ output: 2, user: user(2), deafened: true }),
    ).toBe(0);
  });

  it("treats invalid numbers as 100 %", () => {
    expect(
      effectiveVoiceVolume({ output: NaN, user: user(NaN), deafened: false }),
    ).toBe(1);
  });
});

describe("useUserVolumes", () => {
  it("keeps one participant's level apart from everyone else's", () => {
    useUserVolumes.getState().setVolume("u-bob", 1.4);
    useUserVolumes.getState().setMuted("u-cara", true);
    expect(userVolumeOf("u-bob")).toEqual({ volume: 1.4, muted: false });
    expect(userVolumeOf("u-cara")).toEqual({ volume: 1, muted: true });
    expect(userVolumeOf("u-dora")).toBe(DEFAULT_USER_VOLUME);
    expect(userVolumeOf(null)).toBe(DEFAULT_USER_VOLUME);
  });

  it("clamps to 0–200 % and forgets a participant back at the default", () => {
    useUserVolumes.getState().setVolume("u-bob", 5);
    expect(userVolumeOf("u-bob").volume).toBe(2);
    useUserVolumes.getState().setVolume("u-bob", -1);
    expect(userVolumeOf("u-bob").volume).toBe(0);
    useUserVolumes.getState().setVolume("u-bob", 1);
    expect(useUserVolumes.getState().users).toEqual({});
    useUserVolumes.getState().setVolume("u-bob", 0.3);
    useUserVolumes.getState().setMuted("u-bob", true);
    useUserVolumes.getState().reset("u-bob");
    expect(useUserVolumes.getState().users).toEqual({});
  });

  it("keeps the volume across a local mute", () => {
    useUserVolumes.getState().setVolume("u-bob", 0.6);
    useUserVolumes.getState().setMuted("u-bob", true);
    useUserVolumes.getState().setMuted("u-bob", false);
    expect(userVolumeOf("u-bob")).toEqual({ volume: 0.6, muted: false });
  });

  it("survives a reload through local storage and drops malformed entries", async () => {
    const storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    const first = await import("./userVolume.ts");
    first.useUserVolumes.getState().setVolume("u-bob", 1.7);
    first.useUserVolumes.getState().setMuted("u-cara", true);
    const saved = JSON.parse(storage.items.get("gelabber.user-volumes")!);
    expect(saved).toEqual({
      "u-bob": { volume: 1.7, muted: false },
      "u-cara": { volume: 1, muted: true },
    });

    storage.setItem(
      "gelabber.user-volumes",
      JSON.stringify({
        ...saved,
        "u-dora": "loud",
        "u-emil": { volume: 9, muted: "yes" },
        "u-finn": { volume: 1, muted: false },
      }),
    );
    vi.resetModules();
    const reloaded = await import("./userVolume.ts");
    expect(reloaded.useUserVolumes.getState().users).toEqual({
      "u-bob": { volume: 1.7, muted: false },
      "u-cara": { volume: 1, muted: true },
      "u-emil": { volume: 2, muted: false },
    });
  });

  it("stays usable without storage", async () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("SecurityError");
      },
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
      removeItem: () => undefined,
    });
    const isolated = await import("./userVolume.ts");
    isolated.useUserVolumes.getState().setVolume("u-bob", 0.5);
    expect(isolated.userVolumeOf("u-bob").volume).toBe(0.5);
  });
});
