import { afterEach, describe, expect, it, vi } from "vitest";
import {
  isDesktopApp,
  setNativeBridgeForTests,
  type NativeBridge,
} from "./bridge.ts";
import {
  hasNativeFeature,
  loadNativeFeatures,
  nativeFeatures,
  subscribeNativeFeatures,
} from "./features.ts";

/** A desktop app whose `media_info` answers with `info()`. */
function app(info: () => unknown) {
  const invoke = vi.fn(async (command: string) => {
    if (command !== "media_info") throw new Error(`unexpected ${command}`);
    return info();
  });
  setNativeBridgeForTests({
    invoke,
    channel: async () => null,
  } as unknown as NativeBridge);
  return invoke;
}

const sorted = (features: ReadonlySet<string> | null) =>
  features && [...features].sort();

afterEach(() => {
  setNativeBridgeForTests(undefined);
  vi.unstubAllGlobals();
});

describe("desktop app detection", () => {
  it("wants Tauri's object on the window, not just its name", () => {
    expect(isDesktopApp()).toBe(false);
    vi.stubGlobal("window", {});
    expect(isDesktopApp()).toBe(false);
    vi.stubGlobal("window", { __TAURI_INTERNALS__: null });
    expect(isDesktopApp()).toBe(false);
    vi.stubGlobal("window", { __TAURI_INTERNALS__: "yes" });
    expect(isDesktopApp()).toBe(false);
    vi.stubGlobal("window", { __TAURI_INTERNALS__: { plugins: {} } });
    expect(isDesktopApp()).toBe(true);
  });

  it("asks the real app through Tauri's own invoke", async () => {
    const invoke = vi.fn(async () => ({
      abi: 8,
      version: "0.6.0",
      platform: "windows",
      features: ["camera"],
    }));
    vi.stubGlobal("window", { __TAURI_INTERNALS__: { invoke } });
    expect(sorted(await loadNativeFeatures())).toEqual(["camera"]);
    expect(invoke.mock.calls.map((call) => (call as unknown[])[0])).toEqual([
      "media_info",
    ]);
  });
});

describe("desktop app features", () => {
  it("does nothing in a browser", async () => {
    setNativeBridgeForTests(null);
    const features = nativeFeatures();
    expect(features?.size).toBe(0);
    expect(nativeFeatures()).toBe(features);
    expect(await loadNativeFeatures()).toBe(features);
    expect(hasNativeFeature("screen")).toBe(false);
    expect(hasNativeFeature("camera")).toBe(false);
  });

  it("reads a 0.5.x app, which sends no list, as the Linux app it is", async () => {
    const invoke = app(() => ({ abi: 7, version: "0.5.2", platform: "linux" }));
    // Not known yet: nothing is promised before the app has answered.
    expect(nativeFeatures()).toBeNull();
    expect(hasNativeFeature("screen")).toBe(false);
    expect(sorted(await loadNativeFeatures())).toEqual([
      "app-audio",
      "camera",
      "screen",
    ]);
    expect(hasNativeFeature("screen")).toBe(true);
    expect(hasNativeFeature("camera")).toBe(true);
    expect(hasNativeFeature("app-audio")).toBe(true);
    expect(hasNativeFeature("app-audio-excludes-self")).toBe(false);
    expect(hasNativeFeature("video-frames")).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("takes a newer app's list as it is", async () => {
    app(() => ({ platform: "windows", features: ["camera"] }));
    expect(sorted(await loadNativeFeatures())).toEqual(["camera"]);
    expect(hasNativeFeature("camera")).toBe(true);
    expect(hasNativeFeature("screen")).toBe(false);
    expect(hasNativeFeature("app-audio")).toBe(false);

    app(() => ({ features: [] }));
    expect(sorted(await loadNativeFeatures())).toEqual([]);

    // Names this client does not know yet pass through; junk is dropped.
    app(() => ({
      features: ["screen", "video-frames", "holograms", 7, null, ["camera"]],
    }));
    expect(sorted(await loadNativeFeatures())).toEqual([
      "holograms",
      "screen",
      "video-frames",
    ]);
    expect(hasNativeFeature("video-frames")).toBe(true);
    expect(hasNativeFeature("camera")).toBe(false);
  });

  it("treats an app that cannot answer, or answers oddly, like a 0.5.x one", async () => {
    const legacy = ["app-audio", "camera", "screen"];
    for (const info of [
      () => Promise.reject(new Error("Command media_info not found")),
      () => {
        throw new Error("not allowed by ACL");
      },
      () => null,
      () => undefined,
      () => "0.5.2",
      () => 7,
      () => ({ features: "screen" }),
      () => ({ features: { screen: true } }),
      () => ({ features: null }),
    ]) {
      app(info);
      expect(sorted(await loadNativeFeatures())).toEqual(legacy);
      expect(hasNativeFeature("screen")).toBe(true);
    }
  });

  it("asks once per page and wakes subscribers with the answer", async () => {
    let answer!: (info: unknown) => void;
    const invoke = app(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const woken = vi.fn();
    const unsubscribe = subscribeNativeFeatures(woken);
    const gone = vi.fn();
    subscribeNativeFeatures(gone)();
    expect(nativeFeatures()).toBeNull();
    const first = loadNativeFeatures();
    expect(loadNativeFeatures()).toBe(first);
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(woken).not.toHaveBeenCalled();

    answer({ features: ["screen", "camera", "app-audio-excludes-self"] });
    const features = await first;
    expect(woken).toHaveBeenCalledTimes(1);
    expect(gone).not.toHaveBeenCalled();
    // A stable snapshot for useSyncExternalStore.
    expect(nativeFeatures()).toBe(features);
    expect(nativeFeatures()).toBe(features);
    expect(hasNativeFeature("app-audio-excludes-self")).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(1);
    unsubscribe();
  });
});
