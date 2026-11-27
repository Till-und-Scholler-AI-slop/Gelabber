// The browser's system idle detection (`IdleDetector`, Chromium only) for
// presence (GEL-105): with it, input anywhere on the computer keeps the user
// online, as in the desktop app. Off until the user turns it on in Settings
// → Benachrichtigungen; only that click asks the browser for permission.
// Remembered per browser.

import { create } from "zustand";

import { CLIENT_IDLE_MS } from "../ws/live.ts";
import type { SystemIdleState } from "../ws/idlePresence.ts";

const KEY = "gelabber.presence.systemIdle";

type Detector = EventTarget & {
  userState: "active" | "idle" | null;
  start(options: { threshold: number; signal?: AbortSignal }): Promise<void>;
};
type DetectorClass = {
  new (): Detector;
  requestPermission(): Promise<"granted" | "denied">;
};

function detectorClass(): DetectorClass | null {
  const scope = globalThis as {
    IdleDetector?: unknown;
    isSecureContext?: boolean;
  };
  return typeof scope.IdleDetector === "function" &&
    scope.isSecureContext !== false
    ? (scope.IdleDetector as DetectorClass)
    : null;
}

/** Whether this browser can see input in other programs at all. */
export function systemIdleSupported(): boolean {
  return detectorClass() !== null;
}

type SystemIdleSetting = {
  /** The user turned it on, and the browser allowed it. */
  enabled: boolean;
  /** The last request was refused. */
  denied: boolean;
};

function load(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

function save(enabled: boolean): void {
  try {
    if (enabled) localStorage.setItem(KEY, "1");
    else localStorage.removeItem(KEY);
  } catch {
    /* Storage can be blocked; the switch still works for this tab. */
  }
}

export const useSystemIdle = create<SystemIdleSetting>(() => ({
  enabled: load(),
  denied: false,
}));

function setEnabled(enabled: boolean, denied = false): void {
  save(enabled);
  useSystemIdle.setState({ enabled, denied });
}

/** From the switch's click only: asks the browser for its permission. */
export async function enableSystemIdle(): Promise<void> {
  const Detector = detectorClass();
  if (!Detector) return;
  let answer: "granted" | "denied";
  try {
    answer = await Detector.requestPermission();
  } catch {
    answer = "denied";
  }
  setEnabled(answer === "granted", answer !== "granted");
}

export function disableSystemIdle(): void {
  setEnabled(false);
}

/**
 * Reports the system's idle state, with away after CLIENT_IDLE_MS without
 * input anywhere, until `signal` aborts. Never asks for permission: when it
 * is missing (or was withdrawn) the setting turns itself off and `onState`
 * gets null, so presence falls back to the tabs' own input.
 */
export async function watchSystemIdle(
  onState: (state: SystemIdleState | null) => void,
  signal: AbortSignal,
): Promise<void> {
  const Detector = detectorClass();
  if (!Detector) {
    onState(null);
    return;
  }
  const detector = new Detector();
  const report = () => {
    if (!signal.aborted)
      onState(detector.userState === "idle" ? "idle" : "active");
  };
  detector.addEventListener("change", report);
  try {
    await detector.start({ threshold: CLIENT_IDLE_MS, signal });
  } catch {
    if (signal.aborted) return;
    onState(null);
    disableSystemIdle();
    return;
  }
  report();
}
