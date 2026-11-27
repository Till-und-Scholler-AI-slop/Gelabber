import { afterEach, describe, expect, it, vi } from "vitest";

import { CLIENT_IDLE_MS } from "../ws/live.ts";
import {
  disableSystemIdle,
  enableSystemIdle,
  useSystemIdle,
  watchSystemIdle,
} from "./systemIdle.ts";

class FakeDetector extends EventTarget {
  static permission: "granted" | "denied" = "granted";
  static requested = 0;
  static started: { threshold: number }[] = [];
  static requestPermission() {
    FakeDetector.requested++;
    return Promise.resolve(FakeDetector.permission);
  }
  userState: "active" | "idle" | null = null;
  start(options: { threshold: number }) {
    FakeDetector.started.push(options);
    if (FakeDetector.permission !== "granted")
      return Promise.reject(new DOMException("", "NotAllowedError"));
    this.userState = "active";
    detectors.push(this);
    return Promise.resolve();
  }
}
const detectors: FakeDetector[] = [];

function install() {
  FakeDetector.permission = "granted";
  FakeDetector.requested = 0;
  FakeDetector.started = [];
  detectors.length = 0;
  vi.stubGlobal("IdleDetector", FakeDetector);
}

afterEach(() => {
  disableSystemIdle();
  vi.unstubAllGlobals();
});

describe("system idle detection", () => {
  it("asks for permission only when turned on", async () => {
    install();
    const states: unknown[] = [];
    const watching = new AbortController();
    await watchSystemIdle((state) => states.push(state), watching.signal);
    watching.abort();
    expect(FakeDetector.requested).toBe(0);
    await enableSystemIdle();
    expect(FakeDetector.requested).toBe(1);
    expect(useSystemIdle.getState()).toEqual({ enabled: true, denied: false });
  });

  it("stays off and says so when the browser refuses", async () => {
    install();
    FakeDetector.permission = "denied";
    await enableSystemIdle();
    expect(useSystemIdle.getState()).toEqual({ enabled: false, denied: true });
  });

  it("reports active and idle with the desktop app's five minutes", async () => {
    install();
    const states: unknown[] = [];
    const watching = new AbortController();
    await watchSystemIdle((state) => states.push(state), watching.signal);
    expect(FakeDetector.started).toEqual([
      expect.objectContaining({ threshold: CLIENT_IDLE_MS }),
    ]);
    expect(states).toEqual(["active"]);
    const detector = detectors[0]!;
    detector.userState = "idle";
    detector.dispatchEvent(new Event("change"));
    expect(states).toEqual(["active", "idle"]);
    watching.abort();
    detector.userState = "active";
    detector.dispatchEvent(new Event("change"));
    expect(states).toEqual(["active", "idle"]);
  });

  it("falls back and turns itself off when the permission is gone", async () => {
    install();
    await enableSystemIdle();
    FakeDetector.permission = "denied";
    const states: unknown[] = [];
    await watchSystemIdle(
      (state) => states.push(state),
      new AbortController().signal,
    );
    expect(states).toEqual([null]);
    expect(useSystemIdle.getState().enabled).toBe(false);
  });
});
