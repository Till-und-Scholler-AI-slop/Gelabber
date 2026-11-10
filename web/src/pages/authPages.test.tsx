import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import type { PublicPages } from "../public/pages.ts";

const fixture = vi.hoisted(() => ({
  search: {} as Record<string, unknown>,
  mail: undefined as boolean | undefined,
  links: [] as string[],
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
  }) => {
    fixture.links.push(to);
    return (
      <a
        href={`${to}${search && Object.keys(search).length ? `?${new URLSearchParams(search)}` : ""}`}
        className={className}
      >
        {children}
      </a>
    );
  },
}));
vi.mock("../auth/mail.ts", () => ({
  useAccountMailStatus: () => fixture.mail,
}));

import { AuthFrame } from "./AuthLayout.tsx";
import { LoginPage } from "./LoginPage.tsx";
import { MailSent, RegisterPage } from "./RegisterPage.tsx";

function render(page: ReactNode, publicPages?: PublicPages): string {
  const client = new QueryClient();
  if (publicPages)
    client.setQueryData(["instance", "public-pages"], publicPages);
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>{page}</QueryClientProvider>,
  );
}

const INVITE_BANNER = "Einladung erhalten.";

beforeEach(() => {
  fixture.search = {};
  fixture.mail = undefined;
  fixture.links = [];
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

describe("legal links under the card", () => {
  afterEach(() => setLocale("de", { persist: false }));

  const ALL: PublicPages = {
    landing: false,
    legal: {
      terms: { de: "/de/nutzungsbedingungen", en: "/en/terms" },
      imprint: { de: "/de/impressum", en: "/en/imprint" },
      privacy: { de: "/de/datenschutz", en: "/en/privacy" },
    },
  };
  const frame = (pages?: PublicPages) =>
    render(
      <AuthFrame>
        <section className="lr-auth-card">Karte</section>
      </AuthFrame>,
      pages,
    );

  it("lists the published pages after the card, in the reference's order", () => {
    const html = frame(ALL);
    expect(html).toContain(
      '<section class="lr-auth-card">Karte</section><nav class="auth-legal" aria-label="Rechtliches">' +
        '<a href="/de/impressum">Impressum</a>' +
        '<a href="/de/datenschutz">Datenschutz</a>' +
        '<a href="/de/nutzungsbedingungen">Nutzungsbedingungen</a></nav></div>',
    );
  });

  it("names and links them in the UI language", () => {
    setLocale("en", { persist: false });
    const html = frame(ALL);
    expect(html).toContain(
      '<nav class="auth-legal" aria-label="Legal">' +
        '<a href="/en/imprint">Imprint</a>' +
        '<a href="/en/privacy">Privacy</a>' +
        '<a href="/en/terms">Terms</a></nav>',
    );
  });

  it("shows exactly the published pages", () => {
    const html = frame({
      landing: false,
      legal: { privacy: { de: "/de/datenschutz" } },
    });
    expect(html).toContain(
      '<nav class="auth-legal" aria-label="Rechtliches"><a href="/de/datenschutz">Datenschutz</a></nav>',
    );
    setLocale("en", { persist: false });
    expect(
      frame({ landing: false, legal: { privacy: { de: "/de/datenschutz" } } }),
    ).toContain('<a href="/de/datenschutz">Privacy</a>');
  });

  it("draws no row without a published page", () => {
    expect(frame()).not.toContain("<nav");
    expect(frame({ landing: true, legal: {} })).not.toContain("<nav");
  });

  it("makes the brand a plain link only while the landing page is on", () => {
    // The mocked router Link draws the same anchor, and notes where it led.
    const brand = '<a href="/" class="auth-brand">';
    expect(frame(ALL)).toContain(brand);
    expect(fixture.links).toEqual(["/"]);
    fixture.links = [];
    expect(frame({ ...ALL, landing: true })).toContain(brand);
    expect(fixture.links).toEqual([]);
  });
});
