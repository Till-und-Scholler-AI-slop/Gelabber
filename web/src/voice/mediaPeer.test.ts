import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MediaRetry } from "./mediaPeer.ts";
import { QUIET_MS } from "./testTiming.ts";

/** The largest value the random source gives, just under 1. */
const MAX_JITTER = 1 - Number.EPSILON;

describe("the reconnect backoff", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** The delay of each try until the retry gives up, at this jitter. */
  function delays(jitter: number, retry = new MediaRetry(() => jitter)) {
    const seen: number[] = [];
    let exhausted = false;
    let scheduledAt = Date.now();
    const next = () =>
      retry.schedule(
        () => {
          seen.push(Date.now() - scheduledAt);
          scheduledAt = Date.now();
          next();
        },
        () => {
          exhausted = true;
        },
      );
    next();
    vi.runAllTimers();
    expect(exhausted).toBe(true);
    return seen;
  }

  it("retries soon, then less often, and gives up", () => {
    for (const jitter of [0, 0.5, MAX_JITTER]) {
      const seen = delays(jitter);
      expect(seen.length, `${jitter}`).toBeGreaterThan(1);
      expect(seen[0], `${jitter}`).toBeLessThanOrEqual(300);
      for (let i = 1; i < seen.length; i++)
        expect(seen[i], `${jitter} try ${i}`).toBeGreaterThanOrEqual(
          seen[i - 1]!,
        );
    }
  });

  it("never waits as long as the session tests' quiet time", () => {
    expect(Math.max(...delays(MAX_JITTER))).toBeLessThan(QUIET_MS);
  });

  it("spreads each delay with the jitter", () => {
    const low = delays(0);
    const high = delays(MAX_JITTER);
    expect(high).toHaveLength(low.length);
    low.forEach((delay, i) => expect(high[i]).toBeGreaterThan(delay));
  });

  it("starts over once cancelled", () => {
    const retry = new MediaRetry(() => 0.5);
    const first = delays(0.5, retry);
    retry.cancel();
    expect(retry.active).toBe(false);
    expect(delays(0.5, retry)).toEqual(first);
  });
});
