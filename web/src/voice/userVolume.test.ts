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

  async function signedIn(id: string | null) {
    const { useSession } = await import("../auth/session.ts");
    useSession.setState({
      status: id ? "authenticated" : "anonymous",
      user: id ? ({ id, name: id } as never) : null,
    });
    return useSession;
  }
  /** The store follows the session once every module has loaded. */
  async function loadStore() {
    const module = await import("./userVolume.ts");
    await new Promise((resolve) => setTimeout(resolve, 0));
    return module;
  }

  it("survives a reload through local storage per own account and drops malformed entries", async () => {
    const storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
    vi.resetModules();
    await signedIn("u-me");
    const first = await loadStore();
    first.useUserVolumes.getState().setVolume("u-bob", 1.7);
    first.useUserVolumes.getState().setMuted("u-cara", true);
    // A drag writes once it rests, not on every step.
    expect(storage.items.size).toBe(0);
    first.flushUserVolumes();
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
    vi.resetModules();
    await signedIn("u-me");
    const reloaded = await loadStore();
    expect(reloaded.useUserVolumes.getState().users).toEqual({
      "u-bob": { volume: 1.7, muted: false },
      "u-cara": { volume: 1, muted: true },
      "u-emil": { volume: 2, muted: false },
    });
  });

  it("writes a slider drag once after it rests", async () => {
    try {
      const storage = memoryStorage();
      vi.stubGlobal("localStorage", storage);
      vi.resetModules();
      await signedIn("u-me");
      const writes = vi.spyOn(storage, "setItem");
      const store = await loadStore();
      vi.useFakeTimers();
      for (const volume of [1.1, 1.2, 1.3, 1.4])
        store.useUserVolumes.getState().setVolume("u-bob", volume);
      expect(store.userVolumeOf("u-bob").volume).toBe(1.4);
      expect(writes).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(500);
      expect(writes).toHaveBeenCalledTimes(1);
      // Back at the default, the entry goes away instead of being kept.
      store.useUserVolumes.getState().reset("u-bob");
      await vi.advanceTimersByTimeAsync(500);
      expect(storage.items.has("gelabber.user-volumes:u-me")).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps each own account's levels apart", async () => {
    const storage = memoryStorage({
      "gelabber.user-volumes:u-other": JSON.stringify({
        "u-bob": { volume: 1, muted: true },
      }),
    });
    vi.stubGlobal("localStorage", storage);
    vi.resetModules();
    const session = await signedIn("u-me");
    const store = await loadStore();
    store.useUserVolumes.getState().setVolume("u-bob", 0.4);
    session.setState({
      status: "authenticated",
      user: { id: "u-other", name: "Other" } as never,
    });
    // The change of the first account landed before the switch.
    expect(
      JSON.parse(storage.items.get("gelabber.user-volumes:u-me")!),
    ).toEqual({ "u-bob": { volume: 0.4, muted: false } });
    expect(store.userVolumeOf("u-bob")).toEqual({ volume: 1, muted: true });
    session.setState({ status: "anonymous", user: null });
    expect(store.useUserVolumes.getState().users).toEqual({});
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
    vi.resetModules();
    await signedIn("u-me");
    const isolated = await loadStore();
    isolated.useUserVolumes.getState().setVolume("u-bob", 0.5);
    isolated.flushUserVolumes();
    expect(isolated.userVolumeOf("u-bob").volume).toBe(0.5);
  });
});
