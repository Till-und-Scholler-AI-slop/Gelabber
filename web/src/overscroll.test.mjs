// The root's pull-to-refresh guard and the Tauri app's wheel (#224).
/* global URL */
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("overscroll on the root (index.css)", () => {
  const css = readFileSync(new URL("index.css", import.meta.url), "utf8")
    // Comments may mention the property.
    .replace(/\/\*[\s\S]*?\*\//g, "");
  const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(
    ([, selector, body]) => ({ selector: selector.trim(), body }),
  );

  it("guards pull-to-refresh outside the Tauri app only", () => {
    const root = rules.filter((rule) =>
      /overscroll-behavior(-y)?\s*:\s*none/.test(rule.body),
    );
    expect(root.map((rule) => rule.selector.split(/\s*,\s*/))).toEqual([
      ["html:not([data-tauri])", "html:not([data-tauri]) body"],
    ]);
  });

  it("is not keyed to the pointer, which hybrid devices report either way", () => {
    expect(css).not.toMatch(/@media[^{]*pointer[^{]*\{\s*html\b/);
  });
});
