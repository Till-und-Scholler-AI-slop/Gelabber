/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import defaults from "./defaults.json";
import { builtinTheme } from "./presets.ts";
import { fillThemeColours } from "./themeHtml.ts";

const html = readFileSync(new URL("../../index.html", import.meta.url), "utf8");

describe("default theme colours", () => {
  it("are the colours of the built-in light and dark themes", () => {
    expect(builtinTheme("dark").colors).toEqual(defaults.dark);
    expect(builtinTheme("light").colors).toEqual(defaults.light);
  });

  it("reach index.html only through build-time placeholders", () => {
    for (const mode of ["dark", "light"])
      for (const key of ["background", "text"]) {
        expect(html).toContain(`%theme.${mode}.${key}%`);
        expect(html.toLowerCase()).not.toContain(defaults[mode][key]);
      }
  });

  it("fills every placeholder from defaults.json", () => {
    const filled = fillThemeColours(html);
    expect(filled).not.toMatch(/%theme\./);
    expect(filled).toContain(`content="${defaults.dark.background}"`);
    expect(() => fillThemeColours("%theme.dark.glow%")).toThrow();
  });
});
