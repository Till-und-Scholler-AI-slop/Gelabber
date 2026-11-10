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

import { AppearanceSettings } from "./AppearanceSettings.tsx";

describe("appearance settings", () => {
  it("offers the retry its theme sync message names", () => {
    // The conflict text says "Erneut anwenden"; this page needs the button.
    const html = renderToStaticMarkup(<AppearanceSettings />);
    expect(html).toContain("Erneut anwenden übernimmt sie");
    expect(html).toMatch(/<button[^>]*>Erneut anwenden<\/button>/);
  });
});
