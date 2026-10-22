import { describe, it, expect } from "vitest";
import { BUILTIN_IDS, BUILTIN_THEMES } from "./presets.ts";
import {
  COLOR_KEYS,
  contrast,
  contrastIssues,
  derivedColors,
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
  it.each(["clear", "soft", "terminal"])(
    "rejects a %s style array in imports and cached documents",
    (style) => {
      const theme = importTheme(JSON.stringify(BUILTIN_THEMES[2]));
      const invalid = { ...theme, style: [style] };
      expect(() => importTheme(JSON.stringify(invalid))).toThrow();
      expect(
        isThemeDocument(
          {
            version: 1,
            revision: 1,
            active: theme.id,
            customThemes: [invalid],
          },
          BUILTIN_IDS,
        ),
      ).toBe(false);
    },
  );
  it.each(COLOR_KEYS)(
    "rejects a %s colour array in imports and cached documents",
    (key) => {
      const theme = importTheme(JSON.stringify(BUILTIN_THEMES[2]));
      const invalid = {
        ...theme,
        colors: { ...theme.colors, [key]: [theme.colors[key]] },
      };
      expect(() => importTheme(JSON.stringify(invalid))).toThrow();
      expect(
        isThemeDocument(
          {
            version: 1,
            revision: 1,
            active: theme.id,
            customThemes: [invalid],
          },
          BUILTIN_IDS,
        ),
      ).toBe(false);
    },
  );
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
        // Mid grey: neither dark nor light ink reaches 4.5:1 on it.
        accent: "#7a7a7a",
      },
    };
    expect(contrastIssues(broken)).toHaveLength(3);
    expect(contrastIssues(fixContrast(broken))).toEqual([]);
    expect(broken.colors.text).toBe(original.colors.background);
  });
});

describe("derived interface colours", () => {
  const surfaces = (theme: (typeof BUILTIN_THEMES)[number]) => {
    const d = derivedColors(theme);
    return { c: theme.colors, d };
  };

  it.each(BUILTIN_THEMES.map((theme) => [theme.name, theme] as const))(
    "%s meets the contrast targets of every role",
    (_, theme) => {
      const { c, d } = surfaces(theme);
      const atLeast = (fg: string, bgs: string[], ratio: number) => {
        for (const bg of bgs)
          expect(contrast(fg, bg), `${fg} on ${bg}`).toBeGreaterThanOrEqual(
            ratio,
          );
      };
      atLeast(d.accentInk, [c.accent], 4.5);
      atLeast(
        d.accentText,
        [c.background, c.panel, c.surface, d.selected],
        4.5,
      );
      atLeast(
        d.borderStrong,
        [c.background, c.panel, c.surface, d.elevated],
        3,
      );
      atLeast(d.onlineText, [c.panel, c.surface], 4.5);
      atLeast(d.muted, [c.panel, c.surface, d.hover], 4.5);
      atLeast(d.subtle, [c.panel, d.elevated], 4.5);
      atLeast(d.danger, [c.background, c.panel, c.surface], 4.5);
      atLeast("#ffffff", [d.dangerBg], 4.5);
      for (const tile of d.tiles) atLeast(c.text, [tile], 4.5);
    },
  );

  it("keeps the accent fill as designed, light amber included", () => {
    const light = BUILTIN_THEMES.find((theme) => theme.id === "light")!;
    expect(light.colors.accent).toBe("#e8a638");
    const variables = themeVariables(light);
    expect(variables["--lr-accent"]).toBe("#e8a638");
    expect(variables["--lr-accent-text"]).not.toBe("#e8a638");
    expect(variables["--lr-focus"]).toBe(variables["--lr-accent-text"]);
    for (const theme of BUILTIN_THEMES)
      expect(fixContrast(theme).colors.accent).toBe(theme.colors.accent);
  });

  it("emits one radius scale per style and maps Tailwind onto it", () => {
    const radius = (style: "clear" | "soft" | "terminal") =>
      themeVariables({ ...BUILTIN_THEMES[0], style });
    expect(radius("clear")["--lr-radius-md"]).toBe("7px");
    expect(radius("soft")["--lr-radius-xl"]).toBe("20px");
    expect(radius("terminal")["--lr-radius-lg"]).toBe("2px");
    expect(radius("clear")["--radius-2xl"]).toBe("var(--lr-radius-xl)");
    expect(radius("clear")["--radius-xl"]).toBe("var(--lr-radius-md)");
    expect(radius("clear")["--radius-lg"]).toBe("var(--lr-radius-sm)");
  });

  it("gives every tile token and the backdrop per mode", () => {
    const dark = themeVariables(BUILTIN_THEMES[0]);
    const light = themeVariables(BUILTIN_THEMES[1]);
    for (let i = 1; i <= 8; i++) {
      expect(dark[`--lr-tile-${i}`]).toMatch(/^#[\da-f]{6}$/);
      expect(light[`--lr-tile-${i}`]).toMatch(/^#[\da-f]{6}$/);
    }
    expect(dark["--lr-backdrop"]).not.toBe(light["--lr-backdrop"]);
  });
});
