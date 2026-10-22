// ui.css sits in a cascade layer, so any unlayered rule in the app's other
// stylesheets beats it whatever its specificity. These guard the cases that
// once broke that way.
/* global URL */
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = new URL("../../", import.meta.url);
const read = (path) =>
  readFileSync(path, "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const unlayered = readdirSync(fileURLToPath(src), { recursive: true })
  .filter((file) => file.endsWith(".css") && !file.endsWith("ui.css"))
  .map((file) => ({ file, css: read(new URL(file, src)) }));
const ui = read(new URL("ui.css", import.meta.url));

describe("the primitives' stylesheet", () => {
  it("keeps the shadow of dialogs and sheets: no plain dialog rule sets one", () => {
    const offenders = unlayered.flatMap(({ file, css }) =>
      [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
        .filter(
          ([, selector, body]) =>
            /box-shadow\s*:/.test(body) &&
            selector.split(",").some((part) => /^\s*dialog\s*$/.test(part)),
        )
        .map(([, selector]) => `${file}: ${selector.trim()}`),
    );
    expect(offenders).toEqual([]);
  });

  it("still shows a skeleton when the motion kill switches drop animations", () => {
    for (const scope of [
      /@media \(prefers-reduced-motion: reduce\)\s*\{\s*\.ui-skeleton\s*\{([^}]*)\}/,
      /html\[data-reduced-motion="true"\] \.ui-skeleton\s*\{([^}]*)\}/,
    ]) {
      const body = scope.exec(ui)?.[1] ?? "";
      expect(body).toMatch(/animation:\s*ui-skeleton-in\b[^;]*!important/);
    }
  });
});
