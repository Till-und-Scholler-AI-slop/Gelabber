import { create } from "zustand";

export type ThemePreference = "light" | "dark" | "system";
export type ResolvedTheme = Exclude<ThemePreference, "system">;

export const THEME_STORAGE_KEY = "gelabber.theme";

type ThemeStorage = Pick<Storage, "getItem" | "setItem">;

export function asThemePreference(value: unknown): ThemePreference {
  return value === "light" || value === "dark" || value === "system"
    ? value
    : "system";
}

export function loadThemePreference(storage?: ThemeStorage): ThemePreference {
  const source =
    storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
  if (!source) return "system";
  try {
    return asThemePreference(source.getItem(THEME_STORAGE_KEY));
  } catch {
    return "system";
  }
}

export function saveThemePreference(
  preference: ThemePreference,
  storage?: ThemeStorage,
): void {
  const target =
    storage ?? (typeof localStorage === "undefined" ? undefined : localStorage);
  if (!target) return;
  try {
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
