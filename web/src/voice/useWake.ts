// Overlaid controls that show while the pointer or keyboard is busy and
// hide after a few idle seconds: fullscreen, the large view, Kino.

import { useCallback, useEffect, useRef, useState } from "react";

import { CONTROLS_IDLE_MS } from "./stage.ts";

/** True for a while after `wake`, then false until the next one. */
export function useWake(initial: boolean): [boolean, () => void] {
  const [awake, setAwake] = useState(initial);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const wake = useCallback(() => {
    setAwake(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setAwake(false), CONTROLS_IDLE_MS);
  }, []);
  useEffect(() => {
    if (initial)
      timer.current = setTimeout(() => setAwake(false), CONTROLS_IDLE_MS);
    return () => clearTimeout(timer.current);
  }, [initial]);
  return [awake, wake];
}
