import { afterEach, describe, expect, it, vi } from "vitest";

import {
  EXIT_TIMEOUT_MS,
  exitFinished,
  mergePresence,
  sameValues,
  type ExitTarget,
  type PresenceEntry,
} from "./presence.ts";

type Fake = {
  target: unknown;
  endTime?: number;
  playState?: AnimationPlayState;
  finished?: Promise<unknown>;
};

/** An element whose getAnimations() reports the given animations. */
function element(make: (self: ExitTarget) => Fake[]): ExitTarget {
  const self: ExitTarget = {};
  self.getAnimations = () =>
    make(self).map(
      ({ target, endTime = 130, playState = "running", finished }) =>
        ({
          playState,
          finished: finished ?? new Promise(() => {}),
          effect: { target, getComputedTiming: () => ({ endTime }) },
        }) as unknown as Animation,
    );
  return self;
}

/** Whether `promise` has settled once pending callbacks ran. */
async function settled(promise: Promise<unknown>): Promise<boolean> {
  let done = false;
  void promise.then(() => (done = true));
  await vi.advanceTimersByTimeAsync(0);
  return done;
}

describe("exitFinished", () => {
  afterEach(() => vi.useRealTimers());

  it("settles at once without an element or animation support", async () => {
    vi.useFakeTimers();
    expect(await settled(exitFinished(null))).toBe(true);
    expect(await settled(exitFinished({}))).toBe(true);
    expect(await settled(exitFinished(element(() => [])))).toBe(true);
  });

  it("waits for the element's own exit, its backdrop included", async () => {
    vi.useFakeTimers();
    let end!: () => void;
    const finished = new Promise<void>((resolve) => (end = resolve));
    const dialog = element((self) => [{ target: self, finished }]);
    const exit = exitFinished(dialog);
    expect(await settled(exit)).toBe(false);
    end();
    expect(await settled(exit)).toBe(true);
  });

  it("settles when a transition is cancelled instead of finishing", async () => {
    vi.useFakeTimers();
    const cancelled = Promise.reject(new DOMException("", "AbortError"));
    cancelled.catch(() => {});
    const exit = exitFinished(
      element((self) => [{ target: self, finished: cancelled }]),
    );
    expect(await settled(exit)).toBe(true);
  });

  it("ignores what plays inside it, loops and finished animations", async () => {
    vi.useFakeTimers();
    const surface = element((self) => [
      { target: {} },
      { target: self, endTime: Infinity },
      { target: self, playState: "finished" },
      { target: self, playState: "idle" },
    ]);
    expect(await settled(exitFinished(surface))).toBe(true);
  });

  it("goes anyway once the timeout has passed", async () => {
    vi.useFakeTimers();
    const stuck = element((self) => [{ target: self }]);
    const exit = exitFinished(stuck);
    await vi.advanceTimersByTimeAsync(EXIT_TIMEOUT_MS - 1);
    expect(await settled(exit)).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(await settled(exit)).toBe(true);
  });
});

describe("mergePresence", () => {
  const key = (item: { id: number }) => item.id;
  const open = (...ids: number[]): PresenceEntry<{ id: number }>[] =>
    ids.map((id) => ({ key: id, item: { id }, state: "open" }));
  const states = (entries: PresenceEntry<unknown>[]) =>
    entries.map((entry) => `${entry.key}:${entry.state}`);

  it("opens what is there", () => {
    expect(states(mergePresence([], [{ id: 1 }, { id: 2 }], key, true))).toEqual(
      ["1:open", "2:open"],
    );
  });

  it("keeps a removed entry, closed, where it was", () => {
    const next = mergePresence(open(1, 2, 3), [{ id: 1 }, { id: 3 }], key, true);
    expect(states(next)).toEqual(["1:open", "2:closed", "3:open"]);
    expect(next[1]!.item).toEqual({ id: 2 });
  });

  it("keeps leaving entries in order, before new ones at the end", () => {
    const next = mergePresence(
      open(1, 2, 3),
      [{ id: 2 }, { id: 4 }],
      key,
      true,
    );
    expect(states(next)).toEqual(["1:closed", "2:open", "3:closed", "4:open"]);
  });

  it("still lists an entry that is leaving until it is taken out", () => {
    const leaving = mergePresence(open(1, 2), [{ id: 2 }], key, true);
    const later = mergePresence(leaving, [{ id: 2 }, { id: 3 }], key, true);
    expect(states(later)).toEqual(["1:closed", "2:open", "3:open"]);
  });

  it("opens an entry that comes back while leaving", () => {
    const leaving = mergePresence(open(1, 2), [{ id: 2 }], key, true);
    expect(states(mergePresence(leaving, [{ id: 1 }, { id: 2 }], key, true)))
      .toEqual(["1:open", "2:open"]);
  });

  it("drops removed entries at once with reduced motion", () => {
    expect(states(mergePresence(open(1, 2), [{ id: 2 }], key, false))).toEqual([
      "2:open",
    ]);
  });
});

describe("sameValues", () => {
  it("compares one by one", () => {
    const node = {};
    expect(sameValues(["a", node], ["a", node])).toBe(true);
    expect(sameValues(["a", node], ["a", {}])).toBe(false);
    expect(sameValues(["a"], ["a", undefined])).toBe(false);
    expect(sameValues([NaN], [NaN])).toBe(true);
  });
});
