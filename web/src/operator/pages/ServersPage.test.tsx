import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type { ServerAdmin, ServerAdminPage, ServerQuery } from "../types.ts";

const fixture = vi.hoisted(() => ({
  result: {} as {
    data?: unknown;
    isError?: boolean;
    isFetching?: boolean;
  },
  queries: [] as unknown[],
  chips: [] as { label: string; options: { value: string; label: string }[] }[],
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
  useOperatorServers: (query: ServerQuery) => {
    fixture.queries.push(query);
    return { refetch: vi.fn(), ...fixture.result };
  },
  useOperatorRefresh: () => vi.fn(),
}));
// The chips' values are the API's filters; record what the page offers.
vi.mock("../../components/ui/ChipGroup.tsx", () => ({
  ChipGroup: (props: {
    label: string;
    options: { value: string; label: string }[];
  }) => {
    fixture.chips.push(props);
    return null;
  },
}));

import { ServersPage } from "./ServersPage.tsx";

const server = (overrides: Partial<ServerAdmin> = {}): ServerAdmin => ({
  id: "s1",
  name: "Filmclub",
  icon_url: null,
  created_at: "2026-10-01T10:00:00Z",
  owner: { id: "ada", name: "Ada", email: "ada@example.test" },
  members: 48,
  storage_bytes: 6_800_000_000,
  directory: "off",
  open_reports: 2,
  blocked_at: null,
  blocked_reason: null,
  ...overrides,
});
const page = (servers: ServerAdmin[], total = servers.length) =>
  ({ servers, total }) satisfies ServerAdminPage;

beforeEach(() => {
  fixture.result = {};
  fixture.queries = [];
  fixture.chips = [];
});
afterEach(() => setLocale("de", { persist: false }));

const render = (q?: string) => renderToStaticMarkup(<ServersPage q={q} />);
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

describe("Server page", () => {
  it("shows the reference's title, search and chips", () => {
    fixture.result = { data: page([]) };
    const html = render();
    expect(html).toContain(">Server</h1>");
    expect(html).toContain(
      "Alle Server auf dieser Instanz. Nur für Betreiber sichtbar.",
    );
    expect(html).toContain('placeholder="Server oder Inhaber suchen"');
    expect(fixture.chips[0]?.options).toEqual([
      { value: "all", label: "Alle" },
      { value: "listed", label: "Im Verzeichnis" },
      { value: "reported", label: "Gemeldet" },
      { value: "blocked", label: "Gesperrt" },
    ]);
  });

  it("asks for all servers, or for the search it was opened with", () => {
    fixture.result = { data: page([]) };
    render();
    expect(fixture.queries.at(-1)).toEqual({
      q: undefined,
      filter: "all",
      offset: undefined,
    });
    render("Filmclub");
    expect(fixture.queries.at(-1)).toEqual({
      q: "Filmclub",
      filter: "all",
      offset: undefined,
    });
  });

  it("draws one row per server with right-aligned numbers", () => {
    fixture.result = {
      data: page([
        server(),
        server({
          id: "s2",
          name: "Spieleabend",
          owner: { id: "tom", name: "Tom", email: "tom@example.test" },
          members: 12,
          storage_bytes: 340_000_000,
          directory: "delisted",
          open_reports: 0,
        }),
      ]),
    };
    const html = render();
    const body = text(html);
    expect(body).toContain(
      "Server Inhaber Mitglieder Speicher Verzeichnis Meldungen Aktionen",
    );
    expect(body).toContain("Fi Filmclub Ada 48 6,8 GB – Nicht im Verzeichnis");
    expect(body).toContain("2 2 offene Meldungen");
    expect(body).toContain("Sp Spieleabend Tom 12 340 MB Entfernt 0");
    expect(html).toContain('title="ada@example.test"');
    expect(html).toContain('aria-label="Aktionen für Filmclub"');
    // Three headers, three cells in each of the two rows.
    expect(html.match(/class="is-number"/g)).toHaveLength(9);
    expect(html).toContain('data-label="Speicher"');
    expect(body).toContain(
      "Speicher zählt nur die Anhänge gesendeter Nachrichten.",
    );
  });

  it("shows a server the owner listed in the directory", () => {
    fixture.result = { data: page([server({ directory: "listed" })]) };
    const html = render();
    expect(html).toContain(
      '<span class="ui-badge ui-badge--neutral">Gelistet</span>',
    );
    expect(text(html)).not.toContain("Nicht im Verzeichnis");
  });

  it("marks a blocked server", () => {
    fixture.result = {
      data: page([server({ blocked_at: "2026-11-01T00:00:00Z" })]),
    };
    expect(render()).toMatch(
      /Filmclub<\/span><span class="ui-badge ui-badge--danger">Gesperrt<\/span>/,
    );
  });

  it("pages through more than 50 servers", () => {
    fixture.result = { data: page([server()], 120) };
    const body = text(render());
    expect(body).toContain("1–50 von 120 Zurück Weiter");
  });

  it("has an empty state and a load error", () => {
    fixture.result = { data: page([]) };
    expect(render()).toContain("Keine Server gefunden.");
    fixture.result = { isError: true };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Die Server konnten nicht geladen werden.");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.result = { data: page([server({ directory: "delisted" })]) };
    const body = text(render());
    expect(body).toContain("Servers");
    expect(body).toContain(
      "Server Owner Members Storage Directory Reports Actions",
    );
    expect(body).toContain("Fi Filmclub Ada 48 6.8 GB Removed");
    expect(fixture.chips.at(-1)?.options.map((o) => o.label)).toEqual([
      "All",
      "In the directory",
      "Reported",
      "Blocked",
    ]);
  });
});
