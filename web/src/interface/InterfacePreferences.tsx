import { useEffect } from "react";
import { useInterfacePreferences } from "./preferences.ts";

export function InterfacePreferences() {
  const compactRooms = useInterfacePreferences((s) => s.compactRooms);
  const reducedMotion = useInterfacePreferences((s) => s.reducedMotion);
  useEffect(() => {
    document.documentElement.dataset.compactRooms = String(compactRooms);
    document.documentElement.dataset.reducedMotion = String(reducedMotion);
  }, [compactRooms, reducedMotion]);
  return null;
}
