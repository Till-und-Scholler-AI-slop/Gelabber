// The desktop app (desktop/ui/src/theme.rs) derives the same tokens from a
// theme's eight colours by hand. Both sides check themselves against
// tokens.fixture.json, so neither can drift from the other unnoticed (#561).
//
// After a deliberate change to model.ts or the presets:
//   UPDATE_THEME_TOKENS=1 npx vitest run src/theme/tokens.test.mjs
// and the Rust test (theme::tests::tokens_match_the_web) then tells whether
// theme.rs follows.
/* global process, URL */
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

import { themeVariables } from "./model.ts";
import { BUILTIN_THEMES } from "./presets.ts";

const FIXTURE = fileURLToPath(
  new URL("./tokens.fixture.json", import.meta.url),
);

/** The tokens both clients compute: hex colours and px radii. */
function tokens() {
  return Object.fromEntries(
    BUILTIN_THEMES.map((theme) => {
      const variables = themeVariables(theme);
      const picked = Object.fromEntries(
        Object.entries(variables)
          .filter(
            ([name, value]) =>
              name.startsWith("--lr-") &&
              (/^#[0-9a-f]{6}$/i.test(value) || /^\d+(\.\d+)?px$/.test(value)),
          )
          .map(([name, value]) => [name, value.toLowerCase()])
          .sort(([a], [b]) => a.localeCompare(b)),
      );
      return [
        theme.id,
        {
          mode: theme.mode,
          style: theme.style,
          colors: theme.colors,
          tokens: picked,
        },
      ];
    }),
  );
}

describe("theme tokens shared with the desktop app", () => {
  it("match tokens.fixture.json", () => {
    const now = tokens();
    if (process.env.UPDATE_THEME_TOKENS) {
      writeFileSync(FIXTURE, `${JSON.stringify(now, null, 2)}\n`);
    }
    const fixture = JSON.parse(readFileSync(FIXTURE, "utf8"));
    expect(now).toEqual(fixture);
  });

  it("match the field tokens public.css copies from the dark theme", () => {
    // The public pages load without the theme runtime, so public.css
    // carries the dark preset's field tokens as literals.
    const css = readFileSync(
      fileURLToPath(new URL("../public/public.css", import.meta.url)),
      "utf8",
    );
    const copied = Object.fromEntries(
      [...css.matchAll(/(--lr-field-[\w-]+):\s*([^;]+);/g)].map(
        ([, name, value]) => [name, value.trim().toLowerCase()],
      ),
    );
    const dark = JSON.parse(readFileSync(FIXTURE, "utf8")).dark.tokens;
    const expected = Object.fromEntries(
      Object.entries(dark).filter(([name]) => name.startsWith("--lr-field-")),
    );
    expect(Object.keys(expected).length).toBeGreaterThan(0);
    expect(copied).toEqual(expected);
  });
});
