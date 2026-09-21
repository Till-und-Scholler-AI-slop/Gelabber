import { useEffect } from "react";

import { applyTheme, useTheme } from "./theme.ts";

export function ThemeController() {
  const preference = useTheme((state) => state.preference);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => applyTheme(preference, media.matches);
    update();
    if (preference !== "system") return undefined;
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [preference]);

  return null;
}
