import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  themes: {
    owner: "u1",
    doc: { version: 1, revision: 1, active: "dark", customThemes: [] },
    pending: { kind: "apply", id: "light" } as unknown,
    status: "conflict",
    error:
      "Auf einem anderen Gerät geändert. Deine Änderung bleibt erhalten. Erneut anwenden übernimmt sie auf den aktuellen Stand.",
  },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#">{children}</a>,
}));
vi.mock("../theme/account.ts", () => ({
  changeTheme: vi.fn(),
  retryThemes: vi.fn(),
  syncThemes: vi.fn(),
  useAccountThemes: () => fixture.themes,
}));

import { setLocale } from "../i18n/locale.ts";
import { AppearanceSettings } from "./AppearanceSettings.tsx";

describe("appearance settings", () => {
  it("offers the retry its theme sync message names", () => {
    // The conflict text says "Erneut anwenden"; this page needs the button.
    const html = renderToStaticMarkup(<AppearanceSettings />);
    expect(html).toContain("Erneut anwenden übernimmt sie");
    expect(html).toMatch(/<button[^>]*>Erneut anwenden<\/button>/);
  });

  it("starts with the language, above the colour scheme", () => {
    const html = renderToStaticMarkup(<AppearanceSettings />);
    expect(html.indexOf("Sprache")).toBeLessThan(html.indexOf("Farbschema"));
    expect(html).toContain('name="ui-language"');
    expect(html).toContain("Nachrichten anderer werden nicht übersetzt.");
  });

  it.each([
    ["de", "Deutsch</span>", "English<small>Englisch</small>"],
    ["en", "Deutsch<small>German</small>", "English</span>"],
  ] as const)(
    "names the other language in %s below its own name",
    (locale, deutsch, english) => {
      setLocale(locale, { persist: false });
      try {
        const html = renderToStaticMarkup(<AppearanceSettings />);
        expect(html).toContain(`<span class="ui-choice-text">${deutsch}`);
        expect(html).toContain(`<span class="ui-choice-text">${english}`);
        expect(html).toMatch(
          new RegExp(
            `name="ui-language" checked=""/><span class="ui-choice-text">${
              locale === "de" ? "Deutsch" : "English"
            }`,
          ),
        );
      } finally {
        setLocale("de", { persist: false });
      }
    },
  );
});
