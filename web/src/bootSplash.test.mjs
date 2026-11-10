// The static splash in index.html says "Verbinde…" until React's first render
// replaces it. When the entry bundle fails to load or run, it has to say so.
/* global URL */
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const inline = html.match(/<script>([\s\S]*?)<\/script>/)[1];

/** Runs the shell's inline script and returns its error listener and the splash. */
function boot() {
  const listeners = [];
  const splash = { attributes: {} };
  splash.setAttribute = (name, value) => (splash.attributes[name] = value);
  class HTMLScriptElement {}
  class ErrorEvent {}
  const context = {
    HTMLScriptElement,
    ErrorEvent,
    addEventListener: (type, listener, capture) =>
      listeners.push({ type, listener, capture }),
    localStorage: { getItem: () => null },
    matchMedia: () => ({ matches: true }),
    document: {
      documentElement: {
        classList: { toggle() {} },
        dataset: {},
        style: { setProperty() {} },
      },
      querySelector: (selector) =>
        selector === ".boot-splash" ? splash : null,
    },
  };
  context.window = context;
  runInNewContext(inline, context);
  const errors = listeners.filter((entry) => entry.type === "error");
  return { errors, splash, HTMLScriptElement, ErrorEvent };
}

describe("boot splash", () => {
  it("has a failure text with a reload next to Verbinde…", () => {
    expect(html).toContain("Gelabber konnte nicht geladen werden.");
    expect(html).toMatch(/<a [^>]*href=""[^>]*>Seite neu laden<\/a>/);
  });

  // It appears on its own, away from focus: a screen reader has to hear it.
  it("announces the failure text as an alert", () => {
    expect(html).toMatch(
      /<p role="alert">\s*Gelabber konnte nicht geladen werden\./,
    );
  });

  it("shows it when the entry script fails to load", () => {
    const { errors, splash, HTMLScriptElement } = boot();
    // Load errors do not bubble: only a capturing listener sees them.
    expect(errors.map((entry) => entry.capture)).toEqual([true]);
    errors[0].listener({ target: new HTMLScriptElement() });
    expect(splash.attributes["data-failed"]).toBe("");
  });

  it("shows it when the entry script throws", () => {
    const { errors, splash, ErrorEvent } = boot();
    errors[0].listener(new ErrorEvent());
    expect(splash.attributes["data-failed"]).toBe("");
  });

  it("ignores an image that fails", () => {
    const { errors, splash } = boot();
    errors[0].listener({ target: {} });
    expect(splash.attributes).toEqual({});
  });
});
