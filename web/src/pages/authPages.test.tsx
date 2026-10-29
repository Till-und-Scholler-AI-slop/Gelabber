import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

const fixture = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  mail: undefined as boolean | undefined,
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
