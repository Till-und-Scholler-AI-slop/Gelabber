// Cascade layer order (#375). The first stylesheet that names a layer fixes
// the order, and the build may link a chunk's CSS before index.css. A sheet
// with its own layer therefore states Tailwind's order before using it.
/* global URL */
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";

const ORDER = "@layer properties, theme, base, components, utilities;";

const src = new URL("./", import.meta.url);
const sheets = readdirSync(src, { recursive: true })
  .filter((name) => name.endsWith(".css"))
  .map((name) => ({
    name,
    css: readFileSync(new URL(name, src), "utf8").replace(
      /\/\*[\s\S]*?\*\//g,
      "",
    ),
  }));

describe("cascade layers", () => {
  it("states the order before a sheet's own layer", () => {
    const layered = sheets.filter(
      ({ css }) =>
        /@layer\s+[\w-]+\s*\{/.test(css) &&
        !css.includes('@import "tailwindcss"'),
    );
    expect(layered.map(({ name }) => name)).toContain("components/ui/ui.css");
    for (const { name, css } of layered)
      expect(css.match(/@layer[^{;]*[{;]/)?.[0], name).toBe(ORDER);
  });

  it("keeps Tailwind's own order", () => {
    const require = createRequire(import.meta.url);
    const tailwind = readFileSync(
      require.resolve("tailwindcss/index.css"),
      "utf8",
    );
    expect(tailwind.match(/@layer[^{;]*;/)?.[0]).toBe(
      ORDER.replace("properties, ", ""),
    );
  });
});
