import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OidcPending } from "../auth/types.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({ token: "" }));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => () => Promise.resolve(),
  Link: ({
    children,
    to,
    className,
  }: {
    children: ReactNode;
    to: string;
    className?: string;
  }) => (
    <a href={to} className={className}>
      {children}
    </a>
  ),
}));
vi.mock("../auth/mail.ts", () => ({ useLinkToken: () => fixture.token }));

import { SignInNamePage } from "./SignInNamePage.tsx";

const TOKEN = "ab".repeat(32);
const LENA: OidcPending = {
  provider: { id: "google", name: "Google" },
  email: "lena@example.test",
  name: "Lena Example",
};

function render(pending?: OidcPending): string {
  const client = new QueryClient();
  if (pending) client.setQueryData(["oidc", "pending", TOKEN], pending);
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <SignInNamePage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  fixture.token = TOKEN;
});
afterEach(() => setLocale("de", { persist: false }));

describe("the name step of a first sign-in", () => {
  it("names the provider and the address, and suggests the provider's name", () => {
    const html = render(LENA);
    expect(html).toContain("<h1>Anzeigenamen wählen</h1>");
    expect(html).toContain(
      "Angemeldet mit Google als <strong>lena@example.test</strong>",
    );
    expect(html).toMatch(/<label[^>]*for="name"[^>]*>Anzeigename<\/label>/);
    expect(html).toMatch(/<input[^>]*id="name"[^>]*value="Lena Example"/);
    expect(html).toMatch(/<button type="submit"[^>]*>Konto anlegen<\/button>/);
  });

  it("starts with an empty name when the provider gave none", () => {
    const html = render({ ...LENA, name: null });
    expect(html).toMatch(/<input[^>]*id="name"[^>]*value=""/);
  });

  it("waits for the step without showing a form", () => {
    const html = render();
    expect(html).not.toContain("<form");
    expect(html).not.toContain("Anmeldung abgelaufen");
  });

  it("sends a visitor without a token back to sign in", () => {
    fixture.token = "";
    const html = render();
    expect(html).toContain('<h1 tabindex="-1">Anmeldung abgelaufen</h1>');
    expect(html).toContain(
      '<a href="/login" class="auth-link self-start">Zur Anmeldung</a>',
    );
    expect(html).not.toContain("<form");
  });

  it("speaks English too", () => {
    setLocale("en", { persist: false });
    const html = render(LENA);
    expect(html).toContain("<h1>Choose a display name</h1>");
    expect(html).toContain(
      "Signed in with Google as <strong>lena@example.test</strong>",
    );
    expect(html).toMatch(/>Create account<\/button>/);
  });
});
