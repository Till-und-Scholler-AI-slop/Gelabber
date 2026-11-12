// The static splash in index.html says "Verbinde…" until React's first render
// replaces it. When the entry bundle fails to load or run, it has to say so.
/* global URL */
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

import { translateIn } from "./i18n/catalog.ts";
import { language } from "./i18n/catalogs/language.ts";
import { detectLocale } from "./i18n/locale.ts";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const offline = readFileSync(
  new URL("../public/offline.html", import.meta.url),
  "utf8",
);
const inline = html.match(/<script>([\s\S]*?)<\/script>/)[1];

/** Runs the shell's inline script and returns its error listener and the splash. */
function boot({ stored = null, languages = ["de-DE"], storage } = {}) {
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
    localStorage: storage ?? {
      getItem: (key) => (key === "gelabber.locale" ? stored : null),
    },
    navigator: { languages, language: languages[0] },
    matchMedia: () => ({ matches: true }),
    document: {
      documentElement: {
        lang: "de",
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
  return {
    errors,
    splash,
    HTMLScriptElement,
    ErrorEvent,
    lang: context.document.documentElement.lang,
  };
}

const escape = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

describe("boot splash", () => {
  it.each(["de", "en"])("has the %s texts from the language catalog", (l) => {
    const text = (key) => escape(translateIn(l, language, key));
    expect(html).toMatch(
      new RegExp(`<span lang="${l}">${text("splash.connecting")}</span>`),
    );
    // A failure text with a reload next to it.
    expect(html).toMatch(
      new RegExp(
        `<span lang="${l}">\\s*${text("splash.failed")}\\s*` +
          `<a href="">${text("splash.reload")}</a>\\s*</span>`,
      ),
    );
  });

  // It appears on its own, away from focus: a screen reader has to hear it.
  it("announces the failure text as an alert", () => {
    expect(html).toMatch(/<p role="alert">\s*<span lang="de">\s*Gelabber/);
  });

  it("shows only the texts of the chosen language", () => {
    expect(html).toContain('html:not([lang="en"]) .boot-splash [lang="en"]');
    expect(html).toContain('html[lang="en"] .boot-splash [lang="de"]');
  });

  it.each([
    [["de-AT"]],
    [["fr-FR", "en-US"]],
    [["fr"]],
    [["en-GB", "de"]],
    [[]],
  ])("picks the language for %j as detectLocale does", (languages) => {
    expect(boot({ languages }).lang).toBe(detectLocale(languages));
  });

  it("prefers the language this device last showed", () => {
    expect(boot({ stored: "en", languages: ["de-DE"] }).lang).toBe("en");
    expect(boot({ stored: "de", languages: ["en-GB"] }).lang).toBe("de");
    expect(boot({ stored: "fr", languages: ["de-DE"] }).lang).toBe("de");
  });

  it("still picks one when the storage is blocked", () => {
    const storage = {
      getItem: () => {
        throw new Error("blocked");
      },
    };
    expect(boot({ storage, languages: ["de-CH"] }).lang).toBe("de");
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

describe("offline page", () => {
  const blocks = Object.fromEntries(
    [...offline.matchAll(/<main lang="(\w+)">([\s\S]*?)<\/main>/g)].map(
      ([, l, body]) => [l, body.replace(/\s+/g, " ")],
    ),
  );

  it.each(["de", "en"])("has the %s texts from the language catalog", (l) => {
    const text = (key) => translateIn(l, language, key);
    expect(blocks[l]).toContain(`<h1>${text("offline.heading")}</h1>`);
    expect(blocks[l]).toContain(`<p> ${text("offline.body")} </p>`);
    expect(blocks[l]).toContain(`>${text("offline.retry")}</a>`);
  });

  it("names the page in both languages", () => {
    expect(offline).toContain(
      `<title>${translateIn("de", language, "offline.title")}</title>`,
    );
    expect(offline).toContain(
      `document.title = "${translateIn("en", language, "offline.title")}"`,
    );
  });

  it("picks the language as the shell does", () => {
    const pick = (source) =>
      source.slice(
        source.indexOf("let locale = null;"),
        source.indexOf("document.documentElement.lang = locale;"),
      );
    expect(pick(offline)).toBe(pick(html));
  });
});
