import { describe, it, expect } from "vitest";
import { BUILTIN_IDS, BUILTIN_THEMES } from "./presets.ts";
import {
  contrast,
  contrastIssues,
  fixContrast,
  importTheme,
  isThemeDocument,
  themeVariables,
} from "./model.ts";
describe("theme catalogue and files", () => {
  it("ships twelve adapted Omarchy palettes plus both original modes", () => {
    expect(BUILTIN_THEMES).toHaveLength(14);
    for (const theme of BUILTIN_THEMES) {
      expect(contrastIssues(theme), theme.name).toEqual([]);
      const variables = themeVariables(theme);
      expect(
        contrast(variables["--lr-accent-ink"], theme.colors.accent),
      ).toBeGreaterThanOrEqual(4.5);
      expect(importTheme(JSON.stringify(theme))).toMatchObject({
        name: theme.name,
        colors: theme.colors,
      });
    }
  });
  it("imports an independent copy and rejects executable/unknown/versioned data", () => {
    const exported = JSON.stringify(BUILTIN_THEMES[2]);
    expect(importTheme(exported).id).not.toBe(importTheme(exported).id);
    for (const invalid of [
      "[]",
      "{",
      JSON.stringify({ ...BUILTIN_THEMES[2], version: 2 }),
      JSON.stringify({ ...BUILTIN_THEMES[2], css: "body{}" }),
      JSON.stringify({
        ...BUILTIN_THEMES[2],
        colors: { ...BUILTIN_THEMES[2].colors, background: "url(https://x)" },
      }),
      " ".repeat(65537),
    ])
      expect(() => importTheme(invalid)).toThrow();
  });
  it("validates references, unique custom IDs, names and limits", () => {
    const theme = importTheme(JSON.stringify(BUILTIN_THEMES[0]));
    const doc = {
      version: 1,
      revision: 1,
      active: theme.id,
      customThemes: [theme],
    };
    expect(isThemeDocument(doc, BUILTIN_IDS)).toBe(true);
    for (const invalid of [
      { ...doc, active: "missing" },
      { ...doc, revision: -1 },
      { ...doc, customThemes: [theme, theme] },
      { ...doc, customThemes: [{ ...theme, id: "dark" }] },
      { ...doc, customThemes: [{ ...theme, name: "x".repeat(61) }] },
    ])
      expect(isThemeDocument(invalid, BUILTIN_IDS)).toBe(false);
  });
  it("repairs low contrast while keeping editable colours scoped", () => {
    const original = BUILTIN_THEMES[0];
    const broken = {
      ...original,
      colors: {
        ...original.colors,
        text: original.colors.background,
        muted: original.colors.background,
        accent: original.colors.background,
      },
    };
    expect(contrastIssues(broken)).toHaveLength(3);
    expect(contrastIssues(fixContrast(broken))).toEqual([]);
    expect(broken.colors.text).toBe(original.colors.background);
  });
});
