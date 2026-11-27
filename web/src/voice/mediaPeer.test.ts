import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { MediaRetry } from "./mediaPeer.ts";

describe("the reconnect backoff", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  /** The delay of each try until the retry gives up, at this jitter. */
  function delays(jitter: number): number[] {
    const retry = new MediaRetry(() => jitter);
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

  it("doubles from 250 ms up to 4 s, then gives up after seven tries", () => {
    expect(delays(0.5)).toEqual([250, 500, 1000, 2000, 4000, 4000, 4000]);
  });

  it("spreads each delay with the jitter, from four fifths of it up", () => {
    expect(delays(0)).toEqual([200, 400, 800, 1600, 3200, 3200, 3200]);
    expect(delays(0.75)).toEqual([275, 550, 1100, 2200, 4400, 4400, 4400]);
  });

  it("starts over once cancelled", () => {
    const retry = new MediaRetry(() => 0.5);
    const tries = vi.fn();
    retry.schedule(tries, () => {});
    vi.advanceTimersByTime(250);
    retry.schedule(tries, () => {});
    retry.cancel();
    expect(retry.active).toBe(false);
    vi.advanceTimersByTime(10_000);
    expect(tries).toHaveBeenCalledTimes(1);
    retry.schedule(tries, () => {});
    vi.advanceTimersByTime(249);
    expect(tries).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(1);
    expect(tries).toHaveBeenCalledTimes(2);
  });
});
