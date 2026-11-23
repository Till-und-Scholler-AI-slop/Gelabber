import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type { Account, AccountPage, AccountQuery } from "../types.ts";

const fixture = vi.hoisted(() => ({
  result: {} as {
    data?: unknown;
    isError?: boolean;
    isFetching?: boolean;
  },
  queries: [] as unknown[],
  chips: [] as { label: string; options: { value: string; label: string }[] }[],
  overview: undefined as { mail: boolean; mail_failing: boolean } | undefined,
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => vi.fn(),
}));
vi.mock("@tanstack/react-query", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-query")>()),
  useQueryClient: () => ({}),
}));
vi.mock("../../dms/queries.ts", () => ({
  findCachedDm: () => undefined,
  useOpenDm: () => ({ mutate: vi.fn() }),
}));
vi.mock("../../auth/store.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "lena" }, operator: true }),
}));
vi.mock("../queries.ts", () => ({
  useOperatorAccounts: (query: AccountQuery) => {
    fixture.queries.push(query);
    return { refetch: vi.fn(), ...fixture.result };
  },
  useOperatorOverview: () => ({ data: fixture.overview }),
  useOperatorRefresh: () => vi.fn(),
}));
vi.mock("../../components/ui/ChipGroup.tsx", () => ({
  ChipGroup: (props: {
    label: string;
    options: { value: string; label: string }[];
  }) => {
    fixture.chips.push(props);
    return null;
  },
}));

import { AccountsPage } from "./AccountsPage.tsx";

const account = (overrides: Partial<Account> = {}): Account => ({
  id: "tom",
  name: "Tom",
  email: "tom@example.test",
  avatar_url: null,
  created_at: "2026-10-01T10:00:00Z",
  status: "active",
  email_verified: true,
  disabled_at: null,
  disabled_reason: null,
  operator: false,
  owned_servers: 2,
  member_servers: 5,
  live_sessions: 1,
  ...overrides,
});
const page = (accounts: Account[], total = accounts.length) =>
  ({ accounts, total }) satisfies AccountPage;

beforeEach(() => {
  fixture.result = {};
  fixture.queries = [];
  fixture.chips = [];
  fixture.overview = undefined;
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<AccountsPage />);
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

describe("Konten page", () => {
  it("shows its title, search and chips", () => {
    fixture.result = { data: page([]) };
    const html = render();
    expect(html).toContain(">Konten</h1>");
    expect(html).toContain('placeholder="Name oder E-Mail suchen"');
    expect(fixture.chips[0]?.options).toEqual([
      { value: "all", label: "Alle" },
      { value: "disabled", label: "Gesperrt" },
      // Pending registrations and accounts from before account mail.
      { value: "unconfirmed", label: "Unbestätigt" },
    ]);
    expect(fixture.queries.at(-1)).toEqual({
      q: undefined,
      filter: "all",
      offset: undefined,
    });
  });

  it("draws one row per account", () => {
    fixture.result = {
      data: page([
        account(),
        account({
          id: "mira",
          name: "Mira",
          email: "mira@example.test",
          status: "disabled",
          disabled_reason: "Spam",
          owned_servers: 0,
          member_servers: 1,
        }),
      ]),
    };
    const html = render();
    const body = text(html);
    expect(body).toContain("Name E-Mail Seit Server Status Aktionen");
    expect(body).toContain(
      "T Tom tom@example.test 01.10.2026 2 / 5 2 eigene, Mitglied in 5 Aktiv",
    );
    expect(body).toContain("Mira mira@example.test");
    expect(html).toContain('title="Grund: Spam"');
    expect(html).toContain(
      'class="ui-badge ui-badge--danger" title="Grund: Spam">Gesperrt',
    );
    expect(html).toContain('aria-label="Aktionen für Tom"');
    expect(html).toContain('data-label="E-Mail"');
  });

  it("marks an operator, whose own row has no menu", () => {
    fixture.result = {
      data: page([account({ id: "lena", name: "Lena", operator: true })]),
    };
    const html = render();
    expect(html).toContain(
      '<span class="ui-badge ui-badge--accent">Betrieb</span>',
    );
    expect(html).not.toContain('aria-label="Aktionen für Lena"');
  });

  it("marks an address that was never confirmed (GEL-40)", () => {
    fixture.result = {
      data: page([account({ status: "unconfirmed", email_verified: false })]),
    };
    const html = render();
    expect(html).toContain(
      'class="ui-badge ui-badge--warning" title="Registriert, als dieser Server keine Mails verschickte. Meldet sich erst nach der Bestätigung wieder an.">Nie bestätigt',
    );
  });

  it("says when the last account mail failed", () => {
    fixture.result = { data: page([account()]) };
    fixture.overview = { mail: true, mail_failing: false };
    expect(render()).not.toContain("operator-notice");
    fixture.overview = { mail: true, mail_failing: true };
    expect(text(render())).toContain(
      "Die letzte Mail ist nicht rausgegangen. Prüfe die SMTP-Einstellungen und das Log der API (account mail failed).",
    );
  });

  it("pages, and has an empty state and a load error", () => {
    fixture.result = { data: page([account()], 75) };
    expect(text(render())).toContain("1–50 von 75 Zurück Weiter");
    fixture.result = { data: page([]) };
    expect(render()).toContain("Keine Konten gefunden.");
    fixture.result = { isError: true };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Die Konten konnten nicht geladen werden.");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.result = { data: page([account({ status: "pending" })]) };
    const body = text(render());
    expect(body).toContain("Accounts");
    expect(body).toContain("Name Email Since Servers Status Actions");
    expect(body).toContain("2 / 5 2 owned, member of 5 Unconfirmed");
    fixture.result = {
      data: page([account({ status: "unconfirmed", email_verified: false })]),
    };
    expect(text(render())).toContain("Never confirmed");
    expect(fixture.chips.at(-1)?.options.map((o) => o.label)).toEqual([
      "All",
      "Disabled",
      "Unconfirmed",
    ]);
  });
});
