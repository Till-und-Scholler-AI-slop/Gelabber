import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DeviceSession } from "../auth/devices.ts";
import type { SignInMethods } from "../auth/types.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({ search: {} as Record<string, unknown> }));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => fixture.search,
  useNavigate: () => () => Promise.resolve(),
  Link: ({
    children,
    to,
    params,
    hash,
    className,
  }: {
    children: ReactNode;
    to: string;
    params?: Record<string, string>;
    hash?: string;
    className?: string;
  }) => (
    <a
      href={`${Object.entries(params ?? {}).reduce(
        (path, [key, value]) => path.replace(`$${key}`, value),
        to,
      )}${hash ? `#${hash}` : ""}`}
      className={className}
    >
      {children}
    </a>
  ),
}));

import { exportKey, type ExportStatus } from "../account/export.ts";
import { sessionsKey } from "../auth/devices.ts";
import { signInMethodsKey } from "../auth/signInMethods.ts";
import {
  AccountDataSection,
  ChangePasswordDialog,
  ConnectProviderDialog,
  DeleteAccountDialog,
  DevicesSection,
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

function wrap(
  children: ReactNode,
  methods?: SignInMethods,
  prepare?: (client: QueryClient) => void,
): string {
  const client = new QueryClient();
  if (methods) client.setQueryData(signInMethodsKey(USER), methods);
  prepare?.(client);
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>{children}</QueryClientProvider>,
  );
}

const section = (methods?: SignInMethods) =>
  wrap(<SignInSection userId={USER} />, methods);

/** The row whose title is `title`, as markup. */
function row(html: string, title: string): string {
  const rows = html.split('<li class="settings-action-row">').slice(1);
  const found = rows.find((r) => r.includes(`>${title}<`));
  if (!found) throw new Error(`no row ${title}`);
  return found.slice(0, found.indexOf("</li>"));
}

/** The text of the row's button, and whether it is disabled, natively or
 * as `aria-disabled` (which keeps it focusable). */
function button(rowHtml: string): { label: string; disabled: boolean } {
  const match = /<button([^>]*)>(?:<[^>]+>)*([^<]+)/.exec(rowHtml);
  if (!match) throw new Error("no button");
  return {
    label: match[2]!,
    disabled: / (disabled=""|aria-disabled="true")/.test(match[1]!),
  };
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

describe("Sicherheit → Angemeldete Geräte", () => {
  // Local noon, so the calendar day words do not depend on the hour.
  const ago = (days: number) => {
    const now = new Date();
    return new Date(
      now.getFullYear(),
      now.getMonth(),
      now.getDate() - days,
      12,
    ).toISOString();
  };
  const SESSIONS: DeviceSession[] = [
    {
      id: "00000000-0000-0000-0000-0000000000a1",
      client: "desktop",
      os: "linux",
      created_at: ago(3),
      last_used_at: ago(0),
      current: true,
    },
    {
      id: "00000000-0000-0000-0000-0000000000a2",
      client: "firefox",
      os: "windows",
      created_at: ago(20),
      last_used_at: ago(1),
      current: false,
    },
    {
      id: "00000000-0000-0000-0000-0000000000a3",
      client: "desktop",
      os: "windows",
      created_at: ago(30),
      last_used_at: ago(12),
      current: false,
    },
  ];
  const devices = (prepare?: (client: QueryClient) => void) =>
    wrap(<DevicesSection userId={USER} />, undefined, prepare);
  const listed = (sessions: DeviceSession[]) => (client: QueryClient) =>
    client.setQueryData(sessionsKey(USER), sessions);
  const titles = (html: string) =>
    [...html.matchAll(/class="settings-action-row__title">([^<]+)/g)].map(
      (m) => m[1],
    );

  it("lists the devices in the API's order, then Überall abmelden", () => {
    const html = devices(listed(SESSIONS));
    expect(html).toMatch(
      /<h3 id="devices-heading" class="settings-group-title">Angemeldete Geräte<\/h3>/,
    );
    expect(titles(html)).toEqual([
      "Gelabber für Linux",
      "Firefox unter Windows",
      "Gelabber für Windows",
      "Überall abmelden",
    ]);
  });

  it("marks this device and says when the others were last active", () => {
    const html = devices(listed(SESSIONS));
    const here = row(html, "Gelabber für Linux");
    expect(here).toContain(
      '<span class="ui-badge ui-badge--accent">Dieses Gerät</span>',
    );
    expect(here).toContain("Zuletzt aktiv: gerade eben");
    expect(button(here)).toEqual({ label: "Abmelden", disabled: false });
    const firefox = row(html, "Firefox unter Windows");
    expect(firefox).not.toContain("ui-badge");
    expect(firefox).toContain("Zuletzt aktiv: gestern");
    expect(button(firefox)).toEqual({ label: "Abmelden", disabled: false });
    expect(row(html, "Gelabber für Windows")).toContain(
      "Zuletzt aktiv: vor 12 Tagen",
    );
    expect(html.match(/class="ui-badge /g)).toHaveLength(1);
  });

  it("keeps Überall abmelden as a row with its danger button", () => {
    const everywhere = row(devices(listed(SESSIONS)), "Überall abmelden");
    expect(everywhere).toContain(
      "Beendet jede Anmeldung, auch auf diesem Gerät.",
    );
    expect(everywhere).toContain("ui-button--danger-outline");
    expect(button(everywhere).label).toBe("Auf allen Geräten abmelden");
  });

  it("waits for the list with skeleton rows, Überall abmelden ready", () => {
    const html = devices();
    expect(html).toContain("ui-skeleton");
    expect(titles(html)).toEqual(["Überall abmelden"]);
  });

  it("offers to load the list again when it failed", () => {
    const html = devices((client) => {
      // A server render would otherwise show the retry as loading.
      client.setDefaultOptions({ queries: { retryOnMount: false } });
      client
        .getQueryCache()
        .build(client, { queryKey: sessionsKey(USER) })
        .setState({
          status: "error",
          error: new Error("offline"),
          fetchStatus: "idle",
        });
    });
    expect(html).toMatch(
      /role="alert" class="ui-load-error"><p>Deine Geräte konnten nicht geladen werden\.<\/p>/,
    );
    expect(titles(html)).toEqual(["Überall abmelden"]);
  });

  it("speaks English with the interface", () => {
    setLocale("en", { persist: false });
    const html = devices(listed(SESSIONS));
    expect(html).toContain(">Signed-in devices</h3>");
    const here = row(html, "Gelabber for Linux");
    expect(here).toContain(">This device</span>");
    expect(here).toContain("Last active: just now");
    expect(button(here).label).toBe("Sign out");
    expect(row(html, "Firefox on Windows")).toContain("Last active: yesterday");
    expect(button(row(html, "Sign out everywhere")).label).toBe(
      "Sign out on all devices",
    );
  });
});

describe("Sicherheit → Kontodaten", () => {
  const data = (status?: ExportStatus) =>
    wrap(<AccountDataSection userId={USER} />, undefined, (client) => {
      if (status) client.setQueryData(exportKey(USER), status);
    });
  const exportRowOf = (html: string) => row(html, "Daten exportieren");

  it("titles the section, the export first, then Konto löschen", () => {
    const html = data({ export: null, mail: true });
    expect(html).toMatch(
      /<h3 id="account-data-heading" class="settings-group-title">Kontodaten<\/h3>/,
    );
    const titles = [
      ...html.matchAll(/class="settings-action-row__title">([^<]+)/g),
    ].map((m) => m[1]);
    expect(titles).toEqual(["Daten exportieren", "Konto löschen"]);
    const remove = row(html, "Konto löschen");
    expect(remove).toContain(
      "Endgültig. Server, die dir gehören, übergibst oder löschst du vorher.",
    );
    expect(remove).toContain("ui-button--danger-outline");
    expect(button(remove)).toEqual({ label: "Konto löschen", disabled: false });
  });

  it("opens Konto löschen again after Mit Google bestätigen", () => {
    expect(data({ export: null, mail: true })).not.toContain(
      "Konto endgültig löschen?",
    );
    fixture.search = { delete: 1 };
    expect(data({ export: null, mail: true })).toContain(
      "Konto endgültig löschen?",
    );
    fixture.search = { delete: 1, oidc_error: "failed" };
    expect(data({ export: null, mail: true })).not.toContain(
      "Konto endgültig löschen?",
    );
  });

  it("offers a first export", () => {
    const html = exportRowOf(data({ export: null, mail: true }));
    expect(html).toContain(
      "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du bekommst eine E-Mail, sobald es bereitsteht.",
    );
    expect(button(html)).toEqual({
      label: "Export anfordern",
      disabled: false,
    });
  });

  it("disables the button while the archive is built", () => {
    const html = exportRowOf(
      data({
        export: {
          state: "running",
          requested_at: "2026-11-10T18:00:00Z",
          finished_at: null,
          expires_at: null,
          size_bytes: null,
        },
        mail: false,
      }),
    );
    expect(html).toContain("Wird erstellt. Das kann eine Weile dauern.");
    expect(button(html)).toEqual({ label: "Wird erstellt…", disabled: true });
    // Focusable, so the button that asked keeps focus once it reads this.
    expect(html).not.toMatch(/<button[^>]* disabled=""/);
  });

  it("links the ready archive as the primary action", () => {
    const html = exportRowOf(
      data({
        export: {
          state: "ready",
          requested_at: "2026-11-10T18:00:00Z",
          finished_at: "2026-11-10T18:05:00Z",
          expires_at: "2026-11-17T12:00:00Z",
          size_bytes: 2_621_440,
        },
        mail: true,
      }),
    );
    expect(html).toContain("Bereit bis 17. November 2026, 2,5 MB.");
    expect(html).toMatch(
      /<a href="\/api\/me\/export\/download" class="ui-button ui-button--primary ui-button--sm"[^>]*>Herunterladen<\/a>/,
    );
  });

  it("offers to ask again after a failure", () => {
    const html = exportRowOf(
      data({
        export: {
          state: "failed",
          requested_at: "2026-11-10T18:00:00Z",
          finished_at: "2026-11-10T18:05:00Z",
          expires_at: null,
          size_bytes: null,
        },
        mail: true,
      }),
    );
    expect(html).toContain("Der Export ist fehlgeschlagen.");
    expect(button(html).label).toBe("Erneut anfordern");
  });

  it("waits for the export state, Konto löschen ready", () => {
    const html = data();
    expect(html).toContain("ui-skeleton");
    expect(html).not.toContain(">Daten exportieren<");
    expect(button(row(html, "Konto löschen")).label).toBe("Konto löschen");
  });

  it("speaks English with the interface", () => {
    setLocale("en", { persist: false });
    const html = data({ export: null, mail: false });
    expect(html).toContain(">Account data</h3>");
    const exported = row(html, "Export data");
    expect(exported).toContain(
      "Your messages, files and account data as an archive. You can download it here as soon as it is ready.",
    );
    expect(button(exported).label).toBe("Request export");
    expect(button(row(html, "Delete account")).label).toBe("Delete account");
  });
});

describe("the dialog Konto löschen", () => {
  const SERVER_ID = "00000000-0000-0000-0000-0000000000b1";

  const dialog = (
    methods: SignInMethods,
    prepare?: (client: QueryClient) => void,
    initialOwned?: { id: string; name: string; member_count?: number }[],
  ) =>
    wrap(
      <DeleteAccountDialog
        userId={USER}
        open
        onClose={() => {}}
        initialOwned={initialOwned ?? null}
      />,
      methods,
      prepare,
    );

  it("asks for the password when no server is in the way", () => {
    const html = dialog({ password: true, providers: [] });
    expect(html).toContain(">Konto endgültig löschen?</h2>");
    expect(html).toContain(
      "Deine Nachrichten, Dateien, Direktnachrichten und Reaktionen werden gelöscht. Du verlässt alle Server. Das lässt sich nicht rückgängig machen.",
    );
    expect(html).toContain(
      "Exportiere deine Daten vorher, wenn du sie behalten willst.",
    );
    expect(html).toMatch(/for="delete_password"[^>]*>Passwort/);
    expect(html).toMatch(
      /autoComplete="current-password"|autocomplete="current-password"/,
    );
    expect(html).toMatch(/<button type="button"[^>]*>Abbrechen<\/button>/);
    expect(html).toMatch(
      /<button type="submit" class="ui-button ui-button--danger[^"]*"[^>]*>Konto löschen<\/button>/,
    );
  });

  it("asks an account without a password for its address", () => {
    const html = dialog({ password: false, providers: [CONNECTED] });
    expect(html).not.toContain('for="delete_password"');
    expect(html).toMatch(/for="delete_email"[^>]*>E-Mail-Adresse/);
    expect(html).toContain('type="email"');
    expect(html).toContain(
      "Dein Konto hat kein Passwort. Gib zur Bestätigung seine E-Mail-Adresse ein.",
    );
  });

  it("offers a fresh sign-in through the provider after reauth_required", () => {
    const html = wrap(
      <DeleteAccountDialog
        userId={USER}
        open
        onClose={() => {}}
        initialReauth
      />,
      { password: false, providers: [CONNECTED] },
    );
    expect(html).toContain(
      "Bitte bestätige zuerst, dass du es bist, und melde dich noch einmal an.",
    );
    expect(html).toMatch(/<button type="button"[^>]*>Mit Google bestätigen/);
  });

  it("lists the servers in the way, each with a way to its settings", () => {
    // As from the server list, which has no member count.
    const html = dialog({ password: true, providers: [] }, undefined, [
      { id: SERVER_ID, name: "Wohnzimmer" },
    ]);
    expect(html).toContain(">Konto löschen</h2>");
    expect(html).toContain(
      "Dir gehören noch diese Server. Übergib sie an ein anderes Mitglied oder lösche sie, bevor du dein Konto löschst.",
    );
    const owned = row(html, "Wohnzimmer");
    expect(owned).not.toContain("Mitglied");
    expect(owned).toContain(
      `<a href="/s/${SERVER_ID}/settings#server-settings-gefahrenzone" class="ui-button ui-button--secondary ui-button--sm"`,
    );
    expect(owned).toContain(">Servereinstellungen</a>");
    expect(html).not.toContain('for="delete_password"');
    expect(html).toMatch(/<button type="button"[^>]*>Schließen<\/button>/);
  });

  it("shows the servers of a 409 owns_servers the same way", () => {
    const html = dialog({ password: true, providers: [] }, undefined, [
      { id: "s9", name: "Werkstatt", member_count: 1 },
    ]);
    expect(html).toContain(">Konto löschen</h2>");
    expect(row(html, "Werkstatt")).toContain("1 Mitglied");
  });
});
