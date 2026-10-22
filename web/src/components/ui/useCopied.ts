import { useCallback, useEffect, useRef, useState } from "react";

/** How long a copy control says "Kopiert" before it reads as before. */
export const COPIED_MS = 1500;

/**
 * The "Kopiert" moment of a copy control: `flash()` after a successful
 * copy turns `copied` on for `COPIED_MS`; another copy starts it again.
 */
export function useCopied(): { copied: boolean; flash: () => void } {
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => () => clearTimeout(timer.current), []);
  const flash = useCallback(() => {
    clearTimeout(timer.current);
    setCopied(true);
    timer.current = setTimeout(() => setCopied(false), COPIED_MS);
  }, []);
  return { copied, flash };
}
