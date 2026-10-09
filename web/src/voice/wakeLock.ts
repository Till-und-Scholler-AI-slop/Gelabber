// Keeps a phone's screen on during a call or while watching a stream. The
// browser drops the lock whenever the page is hidden, so it is requested
// again when the page comes back.

import { useEffect } from "react";

/** Holds a screen wake lock on touch devices until the returned function runs. */
export function keepScreenAwake(target: Window): () => void {
  // Missing in the desktop app and in older mobile browsers: nothing to hold.
  if (
    !("wakeLock" in target.navigator) ||
    !target.matchMedia("(pointer: coarse)").matches
  )
    return () => {};
  const page = target.document;
  let lock: WakeLockSentinel | null = null;
  let requesting = false;
  let stopped = false;

  const acquire = () => {
    if (stopped || lock || requesting || page.visibilityState !== "visible")
      return;
    requesting = true;
    target.navigator.wakeLock.request("screen").then(
      (sentinel) => {
        requesting = false;
        if (stopped) {
          void sentinel.release().catch(() => {});
          return;
        }
        lock = sentinel;
        sentinel.addEventListener("release", () => {
          if (lock === sentinel) lock = null;
        });
      },
      () => {
        // Refused, e.g. by a battery saver: the call works without it.
        requesting = false;
      },
    );
  };

  page.addEventListener("visibilitychange", acquire);
  acquire();
  return () => {
    stopped = true;
    page.removeEventListener("visibilitychange", acquire);
    void lock?.release().catch(() => {});
    lock = null;
  };
}

export function useCallWakeLock(active: boolean): void {
  useEffect(() => (active ? keepScreenAwake(window) : undefined), [active]);
}
