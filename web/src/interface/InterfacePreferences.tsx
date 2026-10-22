import { useLayoutEffect } from "react";
import { UI_SCALES, useInterfacePreferences } from "./preferences.ts";

export function InterfacePreferences() {
  const compactRooms = useInterfacePreferences((s) => s.compactRooms);
  const reducedMotion = useInterfacePreferences((s) => s.reducedMotion);
  const uiScale = useInterfacePreferences((s) => s.uiScale);
  // Before the first paint, so a larger text size never jumps in.
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.compactRooms = String(compactRooms);
    root.dataset.reducedMotion = String(reducedMotion);
    root.style.setProperty("--lr-ui-scale", String(UI_SCALES[uiScale]));
  }, [compactRooms, reducedMotion, uiScale]);
  return null;
}
