import { useEffect, useState } from "react";

/** The local calendar day, e.g. "2026-9-22" (month counted from 0). */
export function localDay(now: Date): string {
  return `${now.getFullYear()}-${now.getMonth()}-${now.getDate()}`;
}

/** Milliseconds from `now` until the next local day begins. */
export function msUntilNextDay(now: Date): number {
  const next = new Date(now);
  next.setHours(24, 0, 0, 0);
  return next.getTime() - now.getTime();
}

/**
 * The local day, re-rendering the caller once it changes, so "Heute",
 * "Gestern" and the dates beside the times move on in a tab left open
 * overnight. A sleeping device can hold the timer past midnight, so the
 * day is also checked when the tab is shown again.
 */
export function useLocalDay(): string {
  const [day, setDay] = useState(() => localDay(new Date()));
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const check = () => {
      clearTimeout(timer);
      setDay(localDay(new Date()));
      // A second late, so the timer cannot fire just before midnight.
      timer = setTimeout(check, msUntilNextDay(new Date()) + 1000);
    };
    timer = setTimeout(check, msUntilNextDay(new Date()) + 1000);
    document.addEventListener("visibilitychange", check);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", check);
    };
  }, []);
  return day;
}
