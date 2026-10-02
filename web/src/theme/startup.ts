// Runs before React so a valid cached palette is present on the first paint.
import { isTheme } from "./model.ts";
import { applyDefinition, loadThemePreference } from "./theme.ts";
import { builtinTheme } from "./presets.ts";
try {
  const cached: unknown = JSON.parse(
    localStorage.getItem("gelabber.theme.paint") ?? "null",
  );
  if (
    cached &&
    typeof cached === "object" &&
    "theme" in cached &&
    isTheme(cached.theme)
  ) {
    const system = "system" in cached && cached.system === true;
    applyDefinition(
      system
        ? builtinTheme(
            matchMedia("(prefers-color-scheme: dark)").matches
              ? "dark"
              : "light",
          )
        : cached.theme,
      system,
    );
  } else if (isTheme(cached)) applyDefinition(cached);
  else {
    const preference = loadThemePreference();
    applyDefinition(
      builtinTheme(
        preference === "system"
          ? matchMedia("(prefers-color-scheme: dark)").matches
            ? "dark"
            : "light"
          : preference,
      ),
    );
  }
} catch {
  applyDefinition(builtinTheme("dark"));
}
