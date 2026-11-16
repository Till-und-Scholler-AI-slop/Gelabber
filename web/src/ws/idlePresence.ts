// When this tab counts as away (#566): after CLIENT_IDLE_MS without input
// in the tab, unless the user sits in a voice channel undeafened. The tab
// only sees its own input; the desktop app also counts input elsewhere.
// Free of the gateway and the DOM so the rules are tested on their own, and
// so a manual status (#590) can sit on top of the automatic one.

import { CLIENT_IDLE_MS, PRESENCE_PULSE_MS } from "./live.ts";

/** While in a call, online is repeated this often, so a server that still
 * times sockets out by itself keeps the user online. */
export const CALL_PULSE_MS = 60_000;

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
  /** Pointer or key input in the tab. */
  input(): void;
  /** In a voice channel and not deafened. Ending it (leaving or deafening)
   * starts the five minutes then. */
  setInCall(on: boolean): void;
  stop(): void;
};

export function createIdlePresence(
  send: (st: "o" | "i") => void,
  clock: IdleClock = windowClock,
): IdlePresence {
  let idle = false;
  let inCall = false;
  let lastPulse = -Infinity;
  let idleTimer: unknown = null;
  let callPulse: unknown = null;

  const online = () => {
    idle = false;
    lastPulse = clock.now();
    send("o");
  };
  const armIdle = () => {
    if (idleTimer !== null) clock.clearTimeout(idleTimer);
    idleTimer = inCall
      ? null
      : clock.setTimeout(() => {
          idleTimer = null;
          if (idle || inCall) return;
          idle = true;
          send("i");
        }, CLIENT_IDLE_MS);
  };

  return {
    input() {
      if (idle || clock.now() - lastPulse >= PRESENCE_PULSE_MS) online();
      armIdle();
    },
    setInCall(on) {
      if (on === inCall) return;
      inCall = on;
      if (on) {
        if (idle) online();
        callPulse = clock.setInterval(online, CALL_PULSE_MS);
      } else if (callPulse !== null) {
        clock.clearInterval(callPulse);
        callPulse = null;
      }
      armIdle();
    },
    stop() {
      if (idleTimer !== null) clock.clearTimeout(idleTimer);
      if (callPulse !== null) clock.clearInterval(callPulse);
      idleTimer = callPulse = null;
    },
  };
}
