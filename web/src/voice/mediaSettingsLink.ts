// Every way to the call's settings leads to Settings → Audio or Video (#570):
// the call keeps running and its bar stays in view. The diagnostics are a
// section at the end of Audio.

import { useNavigate } from "@tanstack/react-router";
import { useCallback } from "react";

export type MediaSettingsTarget = "audio" | "video" | "diagnostics";

/** The id of the diagnostics section in Settings → Audio, and the hash that
 * scrolls there. */
export const DIAGNOSTICS_SECTION = "diagnostics";

/** Where `target` lives: the area, and the section's hash for the
 * diagnostics. */
export function mediaSettingsRoute(target: MediaSettingsTarget) {
  return {
    to: "/settings/$area",
    params: { area: target === "video" ? "video" : "audio" },
    hash: target === "diagnostics" ? DIAGNOSTICS_SECTION : undefined,
  } as const;
}

/** Opens Settings on `target`. */
export function useOpenMediaSettings(): (target: MediaSettingsTarget) => void {
  const navigate = useNavigate();
  return useCallback(
    (target) => void navigate(mediaSettingsRoute(target)),
    [navigate],
  );
}
