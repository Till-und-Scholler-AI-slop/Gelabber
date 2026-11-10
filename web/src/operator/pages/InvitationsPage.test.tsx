import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type {
  Invitation,
  InvitationPage,
  InvitationQuery,
  OperatorOverview,
} from "../types.ts";

const fixture = vi.hoisted(() => ({
  overview: undefined as unknown,
  result: {} as {
    data?: unknown;
    isError?: boolean;
    isFetching?: boolean;
  },
  queries: [] as unknown[],
  chips: [] as { label: string; options: { value: string; label: string }[] }[],
}));

vi.mock("../../auth/store.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "lena" }, operator: true }),
}));
vi.mock("../queries.ts", () => ({
  useOperatorOverview: () => ({ data: fixture.overview }),
  useOperatorInvitations: (query: InvitationQuery) => {
    fixture.queries.push(query);
    return { refetch: vi.fn(), ...fixture.result };
  },
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

import { CreatedLink, InvitationsPage } from "./InvitationsPage.tsx";

const invitation = (overrides: Partial<Invitation> = {}): Invitation => ({
  id: "i1",
  code: "k7m2p9qrs4tx",
  link: "https://gelabber.test/register?invitation=k7m2p9qrs4tx",
  email: "ada@example.test",
  note: "Filmclub",
  created_by: { id: "lena", name: "Lena" },
  created_at: "2026-11-01T10:00:00Z",
  expires_at: null,
  status: "open",
  used_by: null,
  used_at: null,
  ...overrides,
});
const page = (invitations: Invitation[], total = invitations.length) =>
  ({ invitations, total }) satisfies InvitationPage;
const overview = (overrides: Partial<OperatorOverview> = {}) =>
  ({
    registration: "invitation",
    mail: true,
    open_reports: 0,
    ...overrides,
  }) satisfies OperatorOverview;

beforeEach(() => {
  fixture.overview = overview();
  fixture.result = {};
  fixture.queries = [];
  fixture.chips = [];
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<InvitationsPage />);
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
const chipLabels = (label: string) =>
  fixture.chips.find((chips) => chips.label === label)?.options;

describe("Beta-Einladungen page", () => {
  it("shows the registration mode", () => {
    fixture.result = { data: page([]) };
    expect(text(render())).toContain("Registrierung: nur mit Einladung");
    fixture.overview = overview({ registration: "open" });
    expect(text(render())).toContain(
      "Registrierung: offen für alle Einladungen füllen dann nur das Formular vor.",
    );
    fixture.overview = undefined;
    expect(render()).not.toContain("Registrierung:");
  });

  it("has the form with seven days chosen and no mail box yet", () => {
    fixture.result = { data: page([]) };
    const html = render();
    expect(html).toContain(">Neue Einladung</h2>");
    expect(html).toContain("E-Mail (optional)");
    expect(html).toContain("Notiz (optional)");
    expect(html).toContain('maxLength="200"');
    expect(chipLabels("Gültig")).toEqual([
      { value: "day1", label: "1 Tag" },
      { value: "day7", label: "7 Tage" },
      { value: "day30", label: "30 Tage" },
      { value: "never", label: "Unbegrenzt" },
    ]);
    // Offered only once an address is entered (invitationActions.test).
    expect(html).not.toContain("Per E-Mail schicken");
    expect(html).toContain(">Einladung erstellen</");
  });

  it("shows a new link ready to copy, and where it was mailed", () => {
    const link = "https://gelabber.test/register?invitation=k7m2p9qrs4tx";
    let html = renderToStaticMarkup(
      <CreatedLink link={link} mailedTo={null} />,
    );
    expect(html).toContain("Neuer Einladungslink");
    expect(html).toContain(`value="${link}"`);
    expect(html).toContain("readOnly");
    expect(text(html)).toContain("Link kopieren");
    expect(html).not.toContain("per E-Mail an");
    html = renderToStaticMarkup(
      <CreatedLink link={link} mailedTo="ada@example.test" />,
    );
    expect(html).toContain(
      "Die Einladung geht per E-Mail an ada@example.test.",
    );
  });

  it("lists invitations by status", () => {
    fixture.result = { data: page([]) };
    render();
    expect(chipLabels("Einladungen filtern")).toEqual([
      { value: "all", label: "Alle" },
      { value: "open", label: "Offen" },
      { value: "used", label: "Benutzt" },
    ]);
    expect(fixture.queries.at(-1)).toEqual({
      status: "all",
      offset: undefined,
    });
  });

  it("draws one row per invitation", () => {
    fixture.result = {
      data: page([
        invitation(),
        invitation({
          id: "i2",
          code: "a2b3c4d5e6f7",
          link: "https://gelabber.test/register?invitation=a2b3c4d5e6f7",
          email: null,
          note: null,
          created_by: null,
          expires_at: "2020-01-01T00:00:00Z",
          status: "used",
          used_by: { id: "ada", name: "Ada" },
        }),
      ]),
    };
    const html = render();
    const body = text(html);
    expect(body).toContain(
      "Link E-Mail Notiz Erstellt von Läuft ab Status Aktionen",
    );
    expect(body).toContain(
      "https://gelabber.test/register?invitation=k7m2p9qrs4tx ada@example.test Filmclub Lena Nie Offen",
    );
    expect(body).toContain("Jede Adresse – keine Angabe Gelöschtes Konto");
    expect(html).toContain('title="Benutzt von Ada"');
    expect(html).toContain(
      'aria-label="Aktionen für die Einladung k7m2p9qrs4tx"',
    );
    // A used invitation has nothing to copy or revoke.
    expect(html).not.toContain("Aktionen für die Einladung a2b3c4d5e6f7");
  });

  it("has an empty state per filter and a load error", () => {
    fixture.result = { data: page([]) };
    expect(render()).toContain("Noch keine Einladungen.");
    fixture.result = { isError: true };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Die Einladungen konnten nicht geladen werden.");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.result = { data: page([invitation()]) };
    const body = text(render());
    expect(body).toContain("Beta invitations");
    expect(body).toContain("Registration: by invitation only");
    expect(body).toContain("New invitation");
    expect(body).toContain("Link Email Note Created by Expires Status Actions");
    expect(body).toContain("Lena Never Open");
  });
});
