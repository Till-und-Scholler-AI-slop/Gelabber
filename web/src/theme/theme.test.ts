import { afterEach, describe, expect, it, vi } from "vitest";

import { builtinTheme } from "./presets.ts";
import {
  THEME_STORAGE_KEY,
  applyDefinition,
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

describe("applied theme", () => {
  afterEach(() => vi.unstubAllGlobals());

  function page() {
    const properties = new Map<string, string>();
    const meta = new Map<string, string>([["content", "#17191a"]]);
    vi.stubGlobal("document", {
      documentElement: {
        classList: { toggle: vi.fn() },
        dataset: {} as Record<string, string>,
        style: {
          colorScheme: "",
          setProperty: (name: string, value: string) =>
            properties.set(name, value),
        },
      },
      querySelector: (selector: string) =>
        selector === 'meta[name="theme-color"]'
          ? {
              setAttribute: (name: string, value: string) =>
                meta.set(name, value),
            }
          : null,
    });
    return { properties, meta };
  }

  it.each(["light", "dark", "catppuccin-latte", "nord"])(
    "paints the browser chrome in the %s page colour",
    (id) => {
      const { properties, meta } = page();
      const theme = builtinTheme(id);
      applyDefinition(theme);
      expect(meta.get("content")).toBe(theme.colors.background);
      expect(meta.get("content")).toBe(properties.get("--lr-bg"));
    },
  );

  it("follows a custom theme and tolerates a page without the meta tag", () => {
    const { meta } = page();
    const custom = {
      ...builtinTheme("dark"),
      id: "custom-1",
      colors: { ...builtinTheme("dark").colors, background: "#102030" },
    };
    applyDefinition(custom);
    expect(meta.get("content")).toBe("#102030");
    vi.stubGlobal("document", {
      ...document,
      querySelector: () => null,
    });
    expect(() => applyDefinition(custom)).not.toThrow();
  });
});
