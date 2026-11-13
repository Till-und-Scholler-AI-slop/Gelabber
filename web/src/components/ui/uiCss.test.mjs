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

  it("gives a field one frame in focus: no offset ring around .ui-field (#561)", () => {
    const index = unlayered.find(({ file }) => file === "index.css").css;
    const body =
      /(?:^|\})\s*\.ui-field:focus-visible\s*\{([^}]*)\}/.exec(index)?.[1] ??
      "";
    expect(body).toMatch(/outline:\s*1px solid transparent/);
    expect(body).toMatch(/outline-offset:\s*0/);
    expect(body).toMatch(/box-shadow:\s*0 0 0 1px var\(--lr-focus\)/);
  });

  it("drops the global ring's page-coloured shadow wherever a field drops its outline (#561)", () => {
    const offenders = [...unlayered, { file: "ui.css", css: ui }].flatMap(
      ({ file, css }) => {
        const rules = [...css.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(
          ([, selector, body]) => ({ selector: selector.trim(), body }),
        );
        // A rule for the same field in focus may reset the shadow instead.
        const resetInFocus = (selector) =>
          rules.some(
            (rule) =>
              rule.selector.startsWith(`${selector}:focus`) &&
              /box-shadow\s*:/.test(rule.body),
          );
        return rules
          .filter(
            ({ selector, body }) =>
              /\b(input|textarea|select)\b/.test(selector) &&
              /(^|;)\s*outline:\s*(0|none)\s*(;|$)/.test(body) &&
              !/box-shadow\s*:/.test(body) &&
              !resetInFocus(selector),
          )
          .map(({ selector }) => `${file}: ${selector}`);
      },
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
