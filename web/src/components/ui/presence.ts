// Surfaces that come and go (popovers, dialogs, toasts, the call bar) stay
// mounted while they leave: marked `data-state="closed"`, they play their
// exit from CSS and unmount once it has run. These are the decisions behind
// that, apart from React and the DOM so they can be tested in node; the
// hooks are in usePresence.ts.

export type PresenceState = "open" | "closed";

/** The longest an exit may take before the surface goes anyway: a hidden
 * tab or a cancelled transition must not keep it on screen. */
export const EXIT_TIMEOUT_MS = 400;

/** What exitFinished needs of an element. */
export type ExitTarget = {
  getAnimations?: (options?: GetAnimationsOptions) => Animation[];
};

/**
 * The exit `element` plays on itself or its `::backdrop`: running, finite
 * transitions and animations. A spinner inside it, or anything looping,
 * is not part of the exit.
 */
function exitAnimations(element: ExitTarget): Animation[] {
  if (typeof element.getAnimations !== "function") return [];
  return element.getAnimations({ subtree: true }).filter((animation) => {
    const effect = animation.effect as KeyframeEffect | null;
    if (!effect || effect.target !== (element as unknown as Element))
      return false;
    if (animation.playState !== "running") return false;
    return Number.isFinite(effect.getComputedTiming().endTime);
  });
}

/**
 * Settles once `element` has played the exit its closed state started, at
 * once when there is none (no element, reduced motion, no animation
 * support), and at the latest after `timeout`. Call it after the closed
 * state is in the DOM: reading the animations starts the transitions.
 */
export function exitFinished(
  element: ExitTarget | null,
  timeout = EXIT_TIMEOUT_MS,
): Promise<void> {
  const running = element ? exitAnimations(element) : [];
  if (running.length === 0) return Promise.resolve();
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, timeout);
    void Promise.allSettled(running.map((animation) => animation.finished)).then(
      () => {
        clearTimeout(timer);
        resolve();
      },
    );
  });
}

export type PresenceKey = string | number;

export type PresenceEntry<T> = {
  key: PresenceKey;
  item: T;
  state: PresenceState;
};

/**
 * The entries a list renders: `items`, open, plus those that just left,
 * closed and in the place they had. One that comes back is open again.
 * Without `animate` (reduced motion) whatever left is gone at once.
 */
export function mergePresence<T>(
  previous: readonly PresenceEntry<T>[],
  items: readonly T[],
  keyOf: (item: T) => PresenceKey,
  animate: boolean,
): PresenceEntry<T>[] {
  const entries: PresenceEntry<T>[] = items.map((item) => ({
    key: keyOf(item),
    item,
    state: "open",
  }));
  if (!animate) return entries;
  const present = new Set(entries.map((entry) => entry.key));
  // Each leaving entry goes right after the one it followed before.
  let at = 0;
  for (const entry of previous) {
    if (present.has(entry.key)) {
      at = entries.findIndex((next) => next.key === entry.key) + 1;
      continue;
    }
    entries.splice(at, 0, { key: entry.key, item: entry.item, state: "closed" });
    at += 1;
  }
  return entries;
}

/** Whether two value lists hold the same values, one by one. */
export function sameValues(
  a: readonly unknown[],
  b: readonly unknown[],
): boolean {
  return a.length === b.length && a.every((value, i) => Object.is(value, b[i]));
}
