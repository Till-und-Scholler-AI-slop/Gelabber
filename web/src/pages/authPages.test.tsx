import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  mail: undefined as boolean | undefined,
  registration: undefined as "open" | "invitation" | undefined,
}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => fixture.search,
  useNavigate: () => () => Promise.resolve(),
  Link: ({
    children,
    to,
    search,
    className,
  }: {
    children: ReactNode;
    to: string;
    search?: Record<string, string>;
    className?: string;
  }) => (
    <a
      href={`${to}${search && Object.keys(search).length ? `?${new URLSearchParams(search)}` : ""}`}
      className={className}
    >
      {children}
    </a>
  ),
}));
vi.mock("../auth/mail.ts", () => ({
  useAccountMailStatus: () => fixture.mail,
}));
vi.mock("../auth/registration.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../auth/registration.ts")>()),
  useRegistrationMode: () => fixture.registration,
}));

import { LoginPage } from "./LoginPage.tsx";
import { MailSent, RegisterPage } from "./RegisterPage.tsx";

function render(page: ReactNode): string {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      {page}
    </QueryClientProvider>,
  );
}

const INVITE_BANNER = "Einladung erhalten.";

beforeEach(() => {
  fixture.search = {};
  fixture.mail = undefined;
  fixture.registration = undefined;
});

describe("login page", () => {
  it("tells an invitee what is going on and offers an account first", () => {
    fixture.search = { redirect: "/invite/k7xm2pqrstuv" };
    const html = render(<LoginPage />);
    expect(html).toContain(INVITE_BANNER);
    expect(html).toMatch(
      /<a href="\/register\?redirect=%2Finvite%2Fk7xm2pqrstuv" class="ui-button ui-button--secondary ui-button--lg[^"]*">Konto anlegen<\/a>/,
    );
    expect(html.indexOf("Konto anlegen")).toBeLessThan(html.indexOf("<form"));
    expect(html).not.toContain("Noch kein Konto?");
  });

  it("keeps the plain sign-in for everyone else", () => {
    const html = render(<LoginPage />);
    expect(html).not.toContain(INVITE_BANNER);
    expect(html).toContain("Noch kein Konto?");
    expect(html).not.toContain('role="alert"');
    expect(html).not.toContain('role="status"');
  });

  it("says why the session ended, without an alert", () => {
    fixture.search = { reason: "expired", redirect: "/s/1" };
    const html = render(<LoginPage />);
    expect(html).toContain(
      '<p role="status" class="auth-banner">Sitzung abgelaufen. Melde dich erneut an.</p>',
    );
    expect(html).not.toContain(INVITE_BANNER);
  });

  it("offers a way back into a forgotten password, mail or not", () => {
    expect(render(<LoginPage />)).toContain('<p class="auth-recovery"></p>');
    fixture.mail = false;
    expect(render(<LoginPage />)).toContain(
      "Passwort vergessen? Frag die Person, die diesen Gelabber-Server betreibt.",
    );
    fixture.mail = true;
    expect(render(<LoginPage />)).toContain(
      '<a href="/forgot-password" class="auth-link">Passwort vergessen?</a>',
    );
  });

  it("lets the password be shown", () => {
    const html = render(<LoginPage />);
    expect(html).toMatch(/aria-label="Passwort anzeigen" aria-pressed="false"/);
  });
});

describe("register page", () => {
  it("shows the invite banner and keeps the invite on the way to sign-in", () => {
    fixture.search = { redirect: "/invite/k7xm2pqrstuv" };
    const html = render(<RegisterPage />);
    expect(html).toContain(INVITE_BANNER);
    expect(html).toContain(
      '<a href="/login?redirect=%2Finvite%2Fk7xm2pqrstuv" class="auth-link">Anmelden</a>',
    );
    expect(html).toMatch(/aria-label="Passwort anzeigen"/);
  });

  it("is a plain form without an invite", () => {
    const html = render(<RegisterPage />);
    expect(html).not.toContain(INVITE_BANNER);
    expect(html).not.toContain('role="alert"');
  });
});

describe("register page on an invitation-only instance", () => {
  const FIELD = /<label[^>]*for="invitation"[^>]*>Einladungscode<\/label>/;
  const ONLY =
    '<p role="status" class="auth-banner">Gelabber ist gerade nur mit Einladung offen.</p>';

  it("asks for a code and says why", () => {
    fixture.registration = "invitation";
    const html = render(<RegisterPage />);
    expect(html).toMatch(FIELD);
    expect(html).toContain(ONLY);
    expect(html).toMatch(/<input[^>]*id="invitation"[^>]*value=""/);
    // The code field comes after the password, before the submit button.
    expect(html.indexOf('id="invitation"')).toBeGreaterThan(
      html.indexOf('id="password"'),
    );
  });

  it("pre-fills the field from the link, open instance or not", () => {
    for (const mode of ["open", "invitation", undefined] as const) {
      fixture.registration = mode;
      fixture.search = { invitation: "k7m2p9qrs4tx" };
      const html = render(<RegisterPage />);
      expect(html).toMatch(FIELD);
      expect(html).toMatch(
        /<input[^>]*id="invitation"[^>]*value="k7m2p9qrs4tx"/,
      );
      expect(html).not.toContain(ONLY);
    }
  });

  it("lets a server invite admit without a code field", () => {
    fixture.registration = "invitation";
    fixture.search = { redirect: "/invite/k7xm2pqrstuv" };
    const html = render(<RegisterPage />);
    expect(html).toContain(INVITE_BANNER);
    expect(html).not.toMatch(FIELD);
    expect(html).not.toContain(ONLY);
  });

  it("shows no field on an open instance or before the mode is known", () => {
    for (const mode of ["open", undefined] as const) {
      fixture.registration = mode;
      const html = render(<RegisterPage />);
      expect(html).not.toContain('id="invitation"');
      expect(html).not.toContain("nur mit Einladung");
    }
  });

  it("says why a sign-in was refused, with the code field", () => {
    fixture.search = { oidc_error: "invitation_required" };
    let html = render(<RegisterPage />);
    expect(html).toContain(
      '<p role="alert" class="auth-banner">Zum Registrieren brauchst du eine Einladung.</p>',
    );
    expect(html).toMatch(FIELD);
    fixture.registration = "invitation";
    fixture.search = { oidc_error: "invitation_invalid", invitation: "abc" };
    html = render(<RegisterPage />);
    expect(html).toContain(
      '<p role="alert" class="auth-banner">Diese Einladung ist unbekannt, abgelaufen oder schon benutzt.</p>',
    );
    expect(html).not.toContain(ONLY);
    expect(html).toMatch(/<input[^>]*id="invitation"[^>]*value="abc"/);
  });

  it("speaks English too", () => {
    setLocale("en", { persist: false });
    try {
      fixture.registration = "invitation";
      const html = render(<RegisterPage />);
      expect(html).toMatch(/>Invitation code<\/label>/);
      expect(html).toContain("Gelabber is open by invitation only for now.");
      fixture.search = { oidc_error: "invitation_invalid" };
      expect(render(<RegisterPage />)).toContain(
        "This invitation is unknown, expired or already used.",
      );
    } finally {
      setLocale("de", { persist: false });
    }
  });
});

describe("mail sent", () => {
  const props = {
    email: "neu@example.org",
    password: "secret-secret",
    onOtherAddress: () => {},
  };

  it("takes focus on its heading and says where the link leads", () => {
    const html = render(
      <MailSent {...props} redirect="/invite/k7xm2pqrstuv" />,
    );
    expect(html).toContain('<h1 tabindex="-1">Postfach prüfen</h1>');
    expect(html).toContain("Danach geht es weiter zur Einladung.");
    expect(render(<MailSent {...props} redirect={undefined} />)).not.toContain(
      "zur Einladung",
    );
  });

  it("waits a minute before it sends the link again", () => {
    const html = render(<MailSent {...props} redirect={undefined} />);
    expect(html).toMatch(
      /<button[^>]*disabled=""[^>]*>Erneut senden \(60 s\)<\/button>/,
    );
    expect(html).toContain(">Andere Adresse verwenden</button>");
    expect(html).toContain('role="status"');
  });
});

describe("in English", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("signs in and registers in English", () => {
    setLocale("en", { persist: false });
    fixture.mail = false;
    const login = render(<LoginPage />);
    expect(login).toContain("<h1>Sign in</h1>");
    expect(login).toContain("No account yet?");
    expect(login).toContain("Ask the person who runs this Gelabber server.");
    expect(login).not.toContain("Anmelden");
    const register = render(<RegisterPage />);
    expect(register).toContain("<h1>Create account</h1>");
    expect(register).toContain("At least 8 characters.");
  });

  it("names the address in bold in the mail-sent text", () => {
    setLocale("en", { persist: false });
    const html = render(
      <MailSent
        email="new@example.org"
        password="secret-secret"
        redirect={undefined}
        onOtherAddress={() => {}}
      />,
    );
    expect(html).toContain(
      "Confirmation link sent to <strong>new@example.org</strong>. It is valid for 24 hours.",
    );
    expect(html).toMatch(/>Send again \(60 s\)<\/button>/);
  });
});
