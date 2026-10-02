import { create } from "zustand";
import { themeVariables, type ThemeDefinition } from "./model.ts";

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = Exclude<ThemePreference, "system">;

export const THEME_STORAGE_KEY = "gelabber.theme";

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

export function asThemePreference(value: unknown): ThemePreference {
  return value === "light" || value === "dark" || value === "system"
    ? value
    : "dark";
}

export function loadThemePreference(storage?: ThemeStorage): ThemePreference {
  try {
    const source =
      storage ??
      (typeof localStorage === "undefined" ? undefined : localStorage);
    if (!source) return "dark";
    return asThemePreference(source.getItem(THEME_STORAGE_KEY));
  } catch {
    return "dark";
  }
}

export function saveThemePreference(
  preference: ThemePreference,
  storage?: ThemeStorage,
): void {
  try {
    const target =
      storage ??
      (typeof localStorage === "undefined" ? undefined : localStorage);
    if (!target) return;
    target.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // Storage can be blocked in private or hardened browser contexts.
  }
}

export function resolveTheme(
  preference: ThemePreference,
  systemDark: boolean,
): ResolvedTheme {
  if (preference === "system") return systemDark ? "dark" : "light";
  return preference;
}

export function applyTheme(
  preference: ThemePreference,
  systemDark = typeof window !== "undefined" &&
    window.matchMedia("(prefers-color-scheme: dark)").matches,
): ResolvedTheme {
  const resolved = resolveTheme(preference, systemDark);
  if (typeof document !== "undefined") {
    document.documentElement.classList.toggle("dark", resolved === "dark");
    document.documentElement.dataset.theme = resolved;
    document.documentElement.style.colorScheme = resolved;
  }
  return resolved;
}

type ThemeState = {
  preference: ThemePreference;
  setPreference: (preference: ThemePreference) => void;
};

export const useTheme = create<ThemeState>((set) => ({
  preference: loadThemePreference(),
  setPreference: (preference) => {
    saveThemePreference(preference);
    applyTheme(preference);
    set({ preference });
  },
}));

export function resetThemeForTests(): void {
  useTheme.setState({ preference: "system" });
}

/** Shared by the app and startup paint cache; only validated theme tokens enter CSS. */
export function applyDefinition(theme: ThemeDefinition, system = false): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  root.classList.toggle("dark", theme.mode === "dark");
  root.dataset.theme = theme.id;
  root.style.colorScheme = theme.mode;
  for (const [name, value] of Object.entries(themeVariables(theme)))
    root.style.setProperty(name, value);
  try {
    localStorage.setItem(
      "gelabber.theme.paint",
      JSON.stringify({ theme, system }),
    );
  } catch {
    /* Optional first-paint cache. */
  }
}
