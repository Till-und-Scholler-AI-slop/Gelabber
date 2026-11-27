import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  BROWSER_IDLE_MS,
  ONLINE_PULSE_MS,
  createIdlePresence,
} from "./idlePresence.ts";
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
  it("without system detection goes away after fifteen minutes, once", () => {
    const { presence, sent } = started();
    vi.advanceTimersByTime(BROWSER_IDLE_MS - 1);
    expect(sent).not.toContain("i");
    // Online is repeated, so the server's own five minutes do not run out.
    expect(sent.length).toBe(
      Math.floor((BROWSER_IDLE_MS - 1) / ONLINE_PULSE_MS) + 1,
    );
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe("i");
    const away = sent.length;
    vi.advanceTimersByTime(BROWSER_IDLE_MS * 3);
    expect(sent.length).toBe(away);
    presence.input();
    expect(sent.at(-1)).toBe("o");
    presence.stop();
  });

  it("counts input in another tab, sending only to come back", () => {
    const { presence, sent } = started();
    for (let minute = 1; minute <= 30; minute++) {
      vi.advanceTimersByTime(60_000);
      presence.otherTabInput();
    }
    expect(sent).not.toContain("i");
    vi.advanceTimersByTime(BROWSER_IDLE_MS);
    expect(sent.at(-1)).toBe("i");
    const away = sent.length;
    presence.otherTabInput();
    expect(sent.length).toBe(away + 1);
    expect(sent.at(-1)).toBe("o");
    // While online, another tab's input sends nothing itself.
    presence.otherTabInput();
    expect(sent.length).toBe(away + 1);
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
    expect(sent.length).toBe(1 + (60 * 60_000) / ONLINE_PULSE_MS);
    presence.stop();
  });

  it("starts the quiet period when the call ends, not before", () => {
    const { presence, sent } = started();
    presence.setInCall(true);
    vi.advanceTimersByTime(30 * 60_000);
    presence.setInCall(false);
    vi.advanceTimersByTime(BROWSER_IDLE_MS - 1);
    expect(sent).not.toContain("i");
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe("i");
    presence.stop();
  });

  it("comes back online when joining a call while away", () => {
    const { presence, sent } = started();
    vi.advanceTimersByTime(BROWSER_IDLE_MS);
    expect(sent.at(-1)).toBe("i");
    presence.setInCall(true);
    expect(sent.at(-1)).toBe("o");
    presence.stop();
  });

  it("sends nothing after stop", () => {
    const { presence, sent } = started();
    presence.setInCall(true);
    presence.stop();
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent).toEqual(["o"]);
  });

  it("follows the system's detection instead of the tab's input", () => {
    const { presence, sent } = started();
    presence.setSystem("active");
    // No input in any tab for an hour, but the user works elsewhere.
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent).not.toContain("i");
    expect(sent.at(-1)).toBe("o");
    // The detection reports five minutes without input anywhere.
    presence.setSystem("idle");
    expect(sent.at(-1)).toBe("i");
    const away = sent.length;
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent.length).toBe(away);
    presence.setSystem("active");
    expect(sent.at(-1)).toBe("o");
    presence.stop();
  });

  it("stays online in a call while the system is idle", () => {
    const { presence, sent } = started();
    presence.setSystem("active");
    presence.setInCall(true);
    presence.setSystem("idle");
    vi.advanceTimersByTime(60 * 60_000);
    expect(sent).not.toContain("i");
    presence.setInCall(false);
    vi.advanceTimersByTime(CLIENT_IDLE_MS - 1);
    expect(sent).not.toContain("i");
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe("i");
    presence.stop();
  });

  it("falls back to the fifteen minutes when the detection stops", () => {
    const { presence, sent } = started();
    presence.setSystem("active");
    vi.advanceTimersByTime(60 * 60_000);
    presence.setSystem(null);
    vi.advanceTimersByTime(BROWSER_IDLE_MS - 1);
    expect(sent).not.toContain("i");
    vi.advanceTimersByTime(1);
    expect(sent.at(-1)).toBe("i");
    presence.stop();
  });

  it("stays within the server's presence frame rate", () => {
    // backend/api/src/limits.rs `WS_PRESENCE`: 30 a minute, bursts of 10.
    const sentAt: number[] = [];
    const presence = createIdlePresence(
      () => sentAt.push(Date.now()),
      fakeClock,
    );
    presence.input();
    presence.setSystem("active");
    presence.setInCall(true);
    for (let ms = 0; ms < 20 * 60_000; ms += 16) {
      vi.advanceTimersByTime(16);
      presence.input();
      presence.otherTabInput();
      if (ms % 90_000 === 0) presence.setSystem("idle");
      if (ms % 90_000 === 48) presence.setSystem("active");
      if (ms % 120_000 === 0) presence.setInCall(ms % 240_000 === 0);
    }
    presence.stop();
    // The server's token bucket: never empty when a frame arrives.
    let tokens = 10;
    let last = sentAt[0] ?? 0;
    for (const at of sentAt) {
      tokens = Math.min(10, tokens + ((at - last) * 30) / 60_000);
      last = at;
      expect(tokens).toBeGreaterThanOrEqual(1);
      tokens -= 1;
    }
    expect(sentAt.length).toBeGreaterThan(100);
  });
});
