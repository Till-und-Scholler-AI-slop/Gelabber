import { useCallback, useEffect, useRef, useState } from "react";

/** How long a "send the mail again" button waits after a mail went out. */
export const RESEND_COOLDOWN_S = 60;

/** Whole seconds from `now` until `deadline` (epoch ms); 0 once passed. */
export function secondsLeft(deadline: number, now: number): number {
  return Math.max(0, Math.ceil((deadline - now) / 1000));
}

/**
 * A wait the user sees count down ("Erneut senden (45 s)"), optionally
 * running from the start. `start` sets it in seconds; `seconds` is what is
 * left, 0 when nothing is pending. It follows the clock, so a throttled
 * background tab still ends on time.
 */
export function useCooldown(initialSeconds = 0) {
  const [initialDeadline] = useState(() => Date.now() + initialSeconds * 1000);
  const deadline = useRef(initialDeadline);
  const [seconds, setSeconds] = useState(() =>
    Math.max(0, Math.ceil(initialSeconds)),
  );
  useEffect(() => {
    if (seconds <= 0) return;
    // Wake just after the next whole second, so each tick drops one.
    const wait = ((deadline.current - Date.now()) % 1000) + 20;
    const timer = setTimeout(
      () => setSeconds(secondsLeft(deadline.current, Date.now())),
      wait > 20 ? wait : 1000,
    );
    return () => clearTimeout(timer);
  }, [seconds]);
  const start = useCallback((wait: number) => {
    deadline.current = Date.now() + wait * 1000;
    setSeconds(secondsLeft(deadline.current, Date.now()));
  }, []);
  return { seconds, start };
}
