import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CALL_PULSE_MS, createIdlePresence } from "./idlePresence.ts";
import { CLIENT_IDLE_MS, PRESENCE_PULSE_MS } from "./live.ts";

const fakeClock = {
  now: () => Date.now(),
  setTimeout: (run: () => void, ms: number) => setTimeout(run, ms),
  clearTimeout: (handle: unknown) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
  setInterval: (run: () => void, ms: number) => setInterval(run, ms),
  clearInterval: (handle: unknown) =>
    clearInterval(handle as ReturnType<typeof setInterval>),
};

function started() {
  const sent: string[] = [];
  const presence = createIdlePresence((st) => sent.push(st), fakeClock);
  presence.input();
  return { presence, sent };
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("idle presence", () => {
  it("goes away after five minutes without input, once", () => {
    const { presence, sent } = started();
    vi.advanceTimersByTime(CLIENT_IDLE_MS - 1);
    expect(sent).toEqual(["o"]);
    vi.advanceTimersByTime(1);
    expect(sent).toEqual(["o", "i"]);
    vi.advanceTimersByTime(CLIENT_IDLE_MS * 3);
    expect(sent).toEqual(["o", "i"]);
    presence.input();
    expect(sent).toEqual(["o", "i", "o"]);
    presence.stop();
  });

  it("pulses on input at most every ten seconds", () => {
    const { presence, sent } = started();
    vi.advanceTimersByTime(PRESENCE_PULSE_MS - 1);
    presence.input();
    expect(sent).toEqual(["o"]);
    vi.advanceTimersByTime(1);
    presence.input();
    expect(sent).toEqual(["o", "o"]);
    presence.stop();
  });

  it("stays online in a call without any input", () => {
    const { presence, sent } = started();
    presence.setInCall(true);
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent).not.toContain("i");
    // The pulse keeps a server that still times out by itself in line.
    expect(sent.length).toBe(1 + (60 * 60_000) / CALL_PULSE_MS);
    presence.stop();
  });

  it("starts the five minutes when the call ends, not before", () => {
    const { presence, sent } = started();
    presence.setInCall(true);
    vi.advanceTimersByTime(30 * 60_000);
    presence.setInCall(false);
    const before = sent.length;
    vi.advanceTimersByTime(CLIENT_IDLE_MS - 1);
    expect(sent.length).toBe(before);
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe("i");
    presence.stop();
  });

  it("comes back online when joining a call while away", () => {
    const { presence, sent } = started();
    vi.advanceTimersByTime(CLIENT_IDLE_MS);
    expect(sent).toEqual(["o", "i"]);
    presence.setInCall(true);
    expect(sent).toEqual(["o", "i", "o"]);
    presence.stop();
  });

  it("sends nothing after stop", () => {
    const { presence, sent } = started();
    presence.setInCall(true);
    presence.stop();
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent).toEqual(["o"]);
  });
});
