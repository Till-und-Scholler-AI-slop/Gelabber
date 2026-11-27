// When this tab counts as away (#566, GEL-105), unless the user sits in a
// voice channel undeafened:
// - with the system's idle detection (the browser's `IdleDetector`, see
//   systemIdle.ts): when it reports no input anywhere on the computer for
//   CLIENT_IDLE_MS, like the desktop app;
// - without it: after BROWSER_IDLE_MS without input in any Gelabber tab of
//   the account. A tab cannot see input in other programs, so it waits
//   longer before it calls someone away.
// Free of the gateway and the DOM so the rules are tested on their own, and
// so a manual status (#590) can sit on top of the automatic one.

import { CLIENT_IDLE_MS, PRESENCE_PULSE_MS } from "./live.ts";

/** Without system idle detection, away after this long without input in
 * any Gelabber tab. */
export const BROWSER_IDLE_MS = 15 * 60 * 1000;

/** While online without input here (in a call, active elsewhere on the
 * computer, or another tab is used), online is repeated this often: the
 * server marks a socket away by itself after five minutes without it. */
export const ONLINE_PULSE_MS = 60_000;

/** What the system's idle detection reports. */
export type SystemIdleState = "active" | "idle";

export type IdleClock = {
  now(): number;
  setTimeout(run: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
  setInterval(run: () => void, ms: number): unknown;
  clearInterval(handle: unknown): void;
};

const windowClock: IdleClock = {
  now: () => Date.now(),
  setTimeout: (run, ms) => window.setTimeout(run, ms),
  clearTimeout: (handle) => window.clearTimeout(handle as number),
  setInterval: (run, ms) => window.setInterval(run, ms),
  clearInterval: (handle) => window.clearInterval(handle as number),
};

export type IdlePresence = {
  /** Pointer or key input in this tab. */
  input(): void;
  /** Input in another Gelabber tab of the same account. Keeps this tab
   * online, but sends only when this tab comes back from away. */
  otherTabInput(): void;
  /** In a voice channel and not deafened. Ending it (leaving or deafening)
   * starts the quiet period then. */
  setInCall(on: boolean): void;
  /** The system's idle state while its detection runs; null without it. */
  setSystem(state: SystemIdleState | null): void;
  stop(): void;
};

export function createIdlePresence(
  send: (st: "o" | "i") => void,
  clock: IdleClock = windowClock,
): IdlePresence {
  let idle = false;
  let inCall = false;
  let system: SystemIdleState | null = null;
  let lastPulse = -Infinity;
  let idleTimer: unknown = null;
  let pulse: unknown = null;

  const pulseOnline = () => {
    lastPulse = clock.now();
    send("o");
  };
  const online = () => {
    idle = false;
    pulseOnline();
    pulse ??= clock.setInterval(() => {
      if (!idle && clock.now() - lastPulse >= ONLINE_PULSE_MS) pulseOnline();
    }, ONLINE_PULSE_MS);
  };
  const away = () => {
    if (idle || inCall || system === "active") return;
    idle = true;
    if (pulse !== null) clock.clearInterval(pulse);
    pulse = null;
    send("i");
  };
  /** Away after `ms`, unless input, a call or the system says otherwise. */
  const armIdle = (ms: number | null) => {
    if (idleTimer !== null) clock.clearTimeout(idleTimer);
    idleTimer =
      inCall || ms === null
        ? null
        : clock.setTimeout(() => {
            idleTimer = null;
            away();
          }, ms);
  };
  // With the system's detection, input only matters as a way back from
  // away: the detection itself says when the computer went quiet.
  const quietPeriod = () => (system === null ? BROWSER_IDLE_MS : null);

  return {
    input() {
      if (idle || clock.now() - lastPulse >= PRESENCE_PULSE_MS) online();
      armIdle(quietPeriod());
    },
    otherTabInput() {
      if (idle) online();
      armIdle(quietPeriod());
    },
    setInCall(on) {
      if (on === inCall) return;
      inCall = on;
      if (on && idle) online();
      armIdle(system === "idle" ? CLIENT_IDLE_MS : quietPeriod());
    },
    setSystem(state) {
      if (state === system) return;
      system = state;
      if (state === "active") {
        if (idle) online();
        armIdle(null);
      } else if (state === "idle") {
        armIdle(null);
        away();
      } else {
        armIdle(BROWSER_IDLE_MS);
      }
    },
    stop() {
      if (idleTimer !== null) clock.clearTimeout(idleTimer);
      if (pulse !== null) clock.clearInterval(pulse);
      idleTimer = pulse = null;
    },
  };
}
