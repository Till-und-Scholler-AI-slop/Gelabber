// public.css writes out the default dark theme's colours, because the
// public pages have no theme runtime; they must stay the theme's values.
/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

import { themeVariables } from "../theme/model.ts";
import { builtinTheme } from "../theme/presets.ts";

describe("public.css", () => {
  it("writes out the default dark theme's values", () => {
    const root = css().match(/:root \{([^}]*)\}/)?.[1] ?? "";
    const theme = themeVariables(builtinTheme("dark"));
    const declared = [...root.matchAll(/(--lr-[\w-]+):\s*([^;]+);/g)];
    expect(declared.length).toBeGreaterThan(10);
    for (const [, name, value] of declared)
      if (name in theme)
        expect(value.replace(/\s+/g, " ").trim(), name).toBe(theme[name]);
  });

  it("moves the feedback form's honeypot off-screen, not out of the page", () => {
    // Some bots skip fields that are display:none (#542).
    const rule = css().match(/\.feedback-hp \{([^}]*)\}/)?.[1] ?? "";
    expect(rule).toContain("position: absolute;");
    expect(rule).toContain("left: -10000px;");
    expect(rule).not.toMatch(/display|visibility/);
  });

  it("has no transition or animation", () => {
    expect(css()).not.toMatch(/\b(transition|animation)\s*:/);
  });
});

function css() {
  return readFileSync(new URL("public.css", import.meta.url), "utf8");
}
