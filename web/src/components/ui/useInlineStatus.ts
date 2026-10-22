import { useCallback, useEffect, useRef, useState } from "react";

export type InlineStatusValue = {
  tone: "success" | "error" | "neutral";
  message: string;
} | null;

/** How long a success or neutral line stays; errors stay until replaced. */
export const INLINE_STATUS_MS = 4000;

/**
 * State for an `InlineStatus`. `show` replaces the line; success and neutral
 * lines clear themselves after a few seconds.
 */
export function useInlineStatus() {
  const [status, setStatus] = useState<InlineStatusValue>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const show = useCallback((next: InlineStatusValue) => {
    clearTimeout(timer.current);
    setStatus(next);
    if (next && next.tone !== "error")
      timer.current = setTimeout(() => setStatus(null), INLINE_STATUS_MS);
  }, []);
  return { status, show, clear: useCallback(() => show(null), [show]) };
}
