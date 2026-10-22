import { afterEach, describe, expect, it, vi } from "vitest";
import { releaseUserScope } from "../auth/release.ts";
import {
  bindUserVolumeAccount,
  DEFAULT_USER_VOLUME,
  flushUserVolumes,
  effectiveVoiceVolume,
  resetUserVolumesForTests,
  syncUserVolumesFromStorage,
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

  it("goes above 100 % and stops at 400 %", () => {
    expect(
      effectiveVoiceVolume({ output: 1, user: user(1.5), deafened: false }),
    ).toBe(1.5);
    expect(
      effectiveVoiceVolume({ output: 1, user: user(3.5), deafened: false }),
    ).toBe(3.5);
    expect(
      effectiveVoiceVolume({ output: 2, user: user(3), deafened: false }),
    ).toBe(4);
    expect(
      effectiveVoiceVolume({ output: 9, user: user(9), deafened: false }),
    ).toBe(4);
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

  it("clamps to 0–400 % and forgets a participant back at the default", () => {
    useUserVolumes.getState().setVolume("u-bob", 5);
    expect(userVolumeOf("u-bob").volume).toBe(4);
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

  it("survives a reload through local storage per own account and drops malformed entries", () => {
    const storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    bindUserVolumeAccount("u-me");
    useUserVolumes.getState().setVolume("u-bob", 1.7);
    useUserVolumes.getState().setMuted("u-cara", true);
    // A drag writes once it rests, not on every step.
    expect(storage.items.size).toBe(0);
    flushUserVolumes();
    const saved = JSON.parse(storage.items.get("gelabber.user-volumes:u-me")!);
    expect(saved).toEqual({
      "u-bob": { volume: 1.7, muted: false },
      "u-cara": { volume: 1, muted: true },
    });

    storage.setItem(
      "gelabber.user-volumes:u-me",
      JSON.stringify({
        ...saved,
        "u-dora": "loud",
        "u-emil": { volume: 9, muted: "yes" },
        "u-finn": { volume: 1, muted: false },
      }),
    );
    // A reload: nothing in memory, the account binds again.
    resetUserVolumesForTests();
    bindUserVolumeAccount("u-me");
    expect(useUserVolumes.getState().users).toEqual({
      "u-bob": { volume: 1.7, muted: false },
      "u-cara": { volume: 1, muted: true },
      "u-emil": { volume: 4, muted: false },
    });
  });

  it("writes a slider drag once after it rests", async () => {
    vi.useFakeTimers();
    try {
      const storage = memoryStorage();
      vi.stubGlobal("localStorage", storage);
      bindUserVolumeAccount("u-me");
      const writes = vi.spyOn(storage, "setItem");
      for (const volume of [1.1, 1.2, 1.3, 1.4])
        useUserVolumes.getState().setVolume("u-bob", volume);
      expect(userVolumeOf("u-bob").volume).toBe(1.4);
      expect(writes).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(500);
      expect(writes).toHaveBeenCalledTimes(1);
      // Back at the default, the entry goes away instead of being kept.
      useUserVolumes.getState().reset("u-bob");
      await vi.advanceTimersByTimeAsync(500);
      expect(storage.items.has("gelabber.user-volumes:u-me")).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("takes over levels another tab saved instead of overwriting them", () => {
    const storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    bindUserVolumeAccount("u-me");
    useUserVolumes.getState().setVolume("u-bob", 1.5);
    flushUserVolumes();
    // Another tab of the same account mutes Cara.
    storage.setItem(
      "gelabber.user-volumes:u-me",
      JSON.stringify({
        "u-bob": { volume: 1.5, muted: false },
        "u-cara": { volume: 1, muted: true },
      }),
    );
    syncUserVolumesFromStorage("gelabber.user-volumes:other-key");
    expect(userVolumeOf("u-cara").muted).toBe(false);
    syncUserVolumesFromStorage("gelabber.user-volumes:u-me");
    expect(userVolumeOf("u-cara").muted).toBe(true);
    // This tab's next change keeps the other tab's mute.
    useUserVolumes.getState().setVolume("u-bob", 0.5);
    flushUserVolumes();
    expect(
      JSON.parse(storage.items.get("gelabber.user-volumes:u-me")!),
    ).toEqual({
      "u-bob": { volume: 0.5, muted: false },
      "u-cara": { volume: 1, muted: true },
    });
  });

  it("keeps its own unsaved change over another tab's write", () => {
    const storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    bindUserVolumeAccount("u-me");
    useUserVolumes.getState().setVolume("u-bob", 1.5);
    storage.setItem("gelabber.user-volumes:u-me", JSON.stringify({}));
    syncUserVolumesFromStorage(null);
    expect(userVolumeOf("u-bob").volume).toBe(1.5);
  });

  it("keeps each own account's levels apart", () => {
    const storage = memoryStorage({
      "gelabber.user-volumes:u-other": JSON.stringify({
        "u-bob": { volume: 1, muted: true },
      }),
    });
    vi.stubGlobal("localStorage", storage);
    bindUserVolumeAccount("u-me");
    useUserVolumes.getState().setVolume("u-bob", 0.4);
    bindUserVolumeAccount("u-other");
    // The change of the first account landed before the switch.
    expect(
      JSON.parse(storage.items.get("gelabber.user-volumes:u-me")!),
    ).toEqual({ "u-bob": { volume: 0.4, muted: false } });
    expect(userVolumeOf("u-bob")).toEqual({ volume: 1, muted: true });
    bindUserVolumeAccount(null);
    expect(useUserVolumes.getState().users).toEqual({});
    // Signed out, a change lasts for the page only.
    useUserVolumes.getState().setVolume("u-bob", 0.2);
    flushUserVolumes();
    expect([...storage.items.keys()]).toEqual([
      "gelabber.user-volumes:u-other",
      "gelabber.user-volumes:u-me",
    ]);
  });

  it("follows every identity change of the tab", () => {
    const storage = memoryStorage({
      "gelabber.user-volumes:u-me": JSON.stringify({
        "u-bob": { volume: 0.3, muted: false },
      }),
    });
    vi.stubGlobal("localStorage", storage);
    releaseUserScope("u-me");
    expect(userVolumeOf("u-bob").volume).toBe(0.3);
    releaseUserScope(null);
    expect(userVolumeOf("u-bob").volume).toBe(1);
  });

  it("stays usable without storage", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("SecurityError");
      },
      setItem: () => {
        throw new Error("QuotaExceededError");
      },
      removeItem: () => undefined,
    });
    bindUserVolumeAccount("u-me");
    useUserVolumes.getState().setVolume("u-bob", 0.5);
    flushUserVolumes();
    expect(userVolumeOf("u-bob").volume).toBe(0.5);
  });
});
