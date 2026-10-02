import { describe, expect, it } from "vitest";

import {
  THEME_STORAGE_KEY,
  asThemePreference,
  loadThemePreference,
  resolveTheme,
  saveThemePreference,
  type ThemePreference,
} from "./theme.ts";

function memoryStorage(initial?: string) {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(THEME_STORAGE_KEY, initial);
  return {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
  };
}

describe("theme preference", () => {
  it("opens the Living Room in dark mode before a preference is saved", () => {
    expect(asThemePreference("sepia")).toBe("dark");
    expect(loadThemePreference(memoryStorage())).toBe("dark");
    expect(loadThemePreference(memoryStorage("invalid"))).toBe("dark");
  });

  it.each<ThemePreference>(["light", "dark", "system"])(
    "persists and restores %s",
    (preference) => {
      const storage = memoryStorage();
      saveThemePreference(preference, storage);
      expect(loadThemePreference(storage)).toBe(preference);
    },
  );

  it("resolves system mode and ignores system changes for explicit modes", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});
