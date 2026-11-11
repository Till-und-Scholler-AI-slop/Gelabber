import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SignInMethods } from "../auth/types.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({ search: {} as Record<string, unknown> }));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => fixture.search,
  useNavigate: () => () => Promise.resolve(),
}));

import { signInMethodsKey } from "../auth/signInMethods.ts";
import {
  ChangePasswordDialog,
  ConnectProviderDialog,
  SetPasswordDialog,
  SignInSection,
} from "./SecurityArea.tsx";

const USER = "00000000-0000-0000-0000-000000000001";
const GOOGLE = { id: "google", name: "Google" };
const CONNECTED = {
  ...GOOGLE,
  configured: true,
  connected: { email: "lena@example.test", since: "2026-11-10T10:00:00Z" },
};
const OPEN = { ...GOOGLE, configured: true, connected: null };

function wrap(children: ReactNode, methods?: SignInMethods): string {
  const client = new QueryClient();
  if (methods) client.setQueryData(signInMethodsKey(USER), methods);
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  );
}

const section = (methods?: SignInMethods) =>
  wrap(<SignInSection userId={USER} />, methods);

/** The row whose title is `title`, as markup. */
function row(html: string, title: string): string {
  const rows = html.split('<li class="settings-action-row">').slice(1);
  const found = rows.find((r) => r.includes(`>${title}</span>`));
  if (!found) throw new Error(`no row ${title}`);
  return found.slice(0, found.indexOf("</li>"));
}

/** The text of the row's button, and whether it is disabled. */
function button(rowHtml: string): { label: string; disabled: boolean } {
  const match = /<button([^>]*)>(?:<[^>]+>)*([^<]+)/.exec(rowHtml);
  if (!match) throw new Error("no button");
  return { label: match[2]!, disabled: / disabled=""/.test(match[1]!) };
}

beforeEach(() => {
  fixture.search = {};
});
afterEach(() => setLocale("de", { persist: false }));

describe("Sicherheit → Anmeldung", () => {
  it("titles the section and waits for the methods without rows", () => {
    const html = section();
    expect(html).toMatch(
      /<h3 id="sign-in-heading" class="settings-group-title">Anmeldung<\/h3>/,
    );
    expect(html).not.toContain("settings-action-row");
  });

  it("offers to change a password and to disconnect a connected provider", () => {
    const html = section({ password: true, providers: [CONNECTED] });
    const password = row(html, "Passwort");
    expect(password).toContain("Zum Anmelden mit deiner E-Mail-Adresse.");
    expect(button(password)).toEqual({ label: "Ändern", disabled: false });
    const google = row(html, "Google");
    expect(google).toContain(
      "Verbunden mit lena@example.test. Trennen geht nur, solange ein Passwort gesetzt ist.",
    );
    expect(button(google)).toEqual({ label: "Trennen", disabled: false });
  });

  it("offers to connect a provider that is not connected", () => {
    const html = section({ password: true, providers: [OPEN] });
    const google = row(html, "Google");
    expect(google).toContain("Nicht verbunden.");
    expect(button(google)).toEqual({ label: "Verbinden", disabled: false });
  });

  it("without a password, offers to set one and keeps the provider", () => {
    const html = section({ password: false, providers: [CONNECTED] });
    const password = row(html, "Passwort");
    expect(password).toContain(
      "Noch keins gesetzt. Du meldest dich mit Google an.",
    );
    expect(button(password)).toEqual({ label: "Festlegen", disabled: false });
    // The hint says why Trennen is off, and the button points to it.
    const google = row(html, "Google");
    expect(button(google)).toEqual({ label: "Trennen", disabled: true });
    const hintId = /<span id="([^"]+-hint)"/.exec(google)?.[1];
    expect(google).toMatch(new RegExp(`aria-describedby="[^"]*${hintId}"`));
  });

  it("lists a provider the operator turned off only to disconnect it", () => {
    const html = section({
      password: true,
      providers: [{ ...CONNECTED, configured: false }],
    });
    expect(button(row(html, "Google")).label).toBe("Trennen");
  });

  it("confirms a connection on the provider's row", () => {
    fixture.search = { connected: "google" };
    const google = row(
      section({ password: true, providers: [CONNECTED] }),
      "Google",
    );
    expect(google).toMatch(
      /role="status" class="ui-inline-status ui-inline-status--success">.*Google ist jetzt verbunden\./,
    );
  });

  it("shows why a connection failed on the row that started it", () => {
    fixture.search = { oidc_error: "identity_taken" };
    const html = section({ password: true, providers: [OPEN] });
    expect(row(html, "Google")).toMatch(
      /ui-inline-status--error">.*Dieses Konto beim Anbieter ist schon mit einem anderen Gelabber-Konto verbunden\./,
    );
    expect(row(html, "Passwort")).not.toContain("ui-inline-status--error");
  });

  it("speaks English with the interface", () => {
    setLocale("en", { persist: false });
    const html = section({ password: false, providers: [CONNECTED] });
    expect(html).toContain(">Sign-in</h3>");
    expect(row(html, "Password")).toContain(
      "None set yet. You sign in with Google.",
    );
    expect(button(row(html, "Google")).label).toBe("Disconnect");
  });
});

describe("the dialogs of Anmeldung", () => {
  it("Ändern holds today's current and new password form", () => {
    const html = wrap(
      <ChangePasswordDialog open onClose={() => {}} onChanged={() => {}} />,
    );
    expect(html).toContain("<h2");
    expect(html).toContain(">Passwort ändern</h2>");
    expect(html).toMatch(/for="current_password"[^>]*>Aktuelles Passwort/);
    expect(html).toMatch(/for="new_password"[^>]*>Neues Passwort/);
    expect(html).toContain("Andere Geräte werden dabei abgemeldet.");
    expect(html).toMatch(
      /<button type="submit"[^>]*>Passwort ändern<\/button>/,
    );
  });

  it("Festlegen asks for the new password only", () => {
    const html = wrap(
      <SetPasswordDialog
        userId={USER}
        provider={GOOGLE}
        open
        onClose={() => {}}
        onSet={() => {}}
      />,
    );
    expect(html).toContain(">Passwort festlegen</h2>");
    expect(html).not.toContain('for="current_password"');
    expect(html).toMatch(/for="new_password"[^>]*>Neues Passwort/);
    expect(html).not.toContain("Mit Google bestätigen");
  });

  it("Festlegen offers to confirm through the provider after reauth_required", () => {
    const html = wrap(
      <SetPasswordDialog
        userId={USER}
        provider={GOOGLE}
        open
        onClose={() => {}}
        onSet={() => {}}
        initialReauth
      />,
    );
    expect(html).toContain(
      "Bitte bestätige zuerst, dass du es bist, und melde dich noch einmal an.",
    );
    expect(html).toMatch(/<button type="button"[^>]*>Mit Google bestätigen/);
  });

  it("Verbinden asks for the current password before leaving", () => {
    const html = wrap(
      <ConnectProviderDialog provider={GOOGLE} open onClose={() => {}} />,
    );
    expect(html).toContain(">Google verbinden</h2>");
    expect(html).toContain(
      "Bestätige mit deinem Passwort. Danach geht es weiter zu Google.",
    );
    expect(html).toMatch(/for="link_current_password"[^>]*>Aktuelles Passwort/);
    expect(html).toMatch(
      /<button type="submit"[^>]*>Weiter zu Google<\/button>/,
    );
  });
});
