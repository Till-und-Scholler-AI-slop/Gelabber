import { useSyncExternalStore } from "react";

import { useInterfacePreferences } from "./preferences.ts";

const QUERY = "(prefers-reduced-motion: reduce)";

function subscribe(onChange: () => void): () => void {
  if (typeof window === "undefined" || !window.matchMedia) return () => {};
  const media = window.matchMedia(QUERY);
  media.addEventListener("change", onChange);
  return () => media.removeEventListener("change", onChange);
}

function systemPrefers(): boolean {
  return (
    typeof window !== "undefined" &&
    Boolean(window.matchMedia) &&
    window.matchMedia(QUERY).matches
  );
}

/** The system setting or Gelabber's own "Weniger Bewegung" switch. */
export function useReducedMotion(): boolean {
  const own = useInterfacePreferences((s) => s.reducedMotion);
  const system = useSyncExternalStore(subscribe, systemPrefers, () => false);
  return own || system;
}
