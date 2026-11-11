import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type { Report, ReportPage, ReportQuery } from "../types.ts";

const fixture = vi.hoisted(() => ({
  result: {} as { data?: unknown; isError?: boolean },
  queries: [] as unknown[],
  chips: [] as { options: { value: string; label: string }[] }[],
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
  useOperatorReports: (query: ReportQuery) => {
    fixture.queries.push(query);
    return { refetch: vi.fn(), ...fixture.result };
  },
  useOperatorRefresh: () => vi.fn(),
}));
vi.mock("../../components/ui/ChipGroup.tsx", () => ({
  ChipGroup: (props: { options: { value: string; label: string }[] }) => {
    fixture.chips.push(props);
    return null;
  },
}));

import { ReportsPage } from "./ReportsPage.tsx";

const report = (overrides: Partial<Report> = {}): Report => ({
  id: "r1",
  server: {
    id: "s1",
    name: "Filmclub",
    icon_url: null,
    blocked: false,
    directory: "off",
  },
  reporter: { id: "tom", name: "Tom" },
  reason: "spam",
  details: "Lauter Werbelinks im Kanal #allgemein.",
  status: "open",
  created_at: new Date(Date.now() - 5 * 60_000).toISOString(),
  resolved_at: null,
  resolved_by: null,
  resolution_note: null,
  ...overrides,
});
const page = (reports: Report[]) =>
  ({ reports, total: reports.length }) satisfies ReportPage;

beforeEach(() => {
  fixture.result = {};
  fixture.queries = [];
  fixture.chips = [];
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<ReportsPage />);
const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();

describe("Meldungen page", () => {
  it("lists open reports first, with Offen and Erledigt", () => {
    fixture.result = { data: page([]) };
    const html = render();
    expect(html).toContain(">Meldungen</h1>");
    expect(fixture.queries.at(-1)).toEqual({
      status: "open",
      offset: undefined,
    });
    expect(fixture.chips[0]?.options).toEqual([
      { value: "open", label: "Offen" },
      { value: "closed", label: "Erledigt" },
    ]);
    expect(html).toContain("Keine offenen Meldungen.");
  });

  it("draws server, reason, details, reporter and time", () => {
    fixture.result = {
      data: page([
        report(),
        report({
          id: "r2",
          reason: "other",
          reporter: null,
          details: "",
          server: {
            id: "s2",
            name: "Spieleabend",
            icon_url: null,
            blocked: true,
            directory: "off",
          },
        }),
      ]),
    };
    const html = render();
    const body = text(html);
    expect(body).toContain("Server Grund Details Gemeldet von Wann Aktionen");
    expect(body).toContain(
      "Fi Filmclub Spam Lauter Werbelinks im Kanal #allgemein. Tom vor 5 Minuten",
    );
    expect(body).toContain(
      "Sp Spieleabend Gesperrt Anderes – keine Angabe Gelöschtes Konto",
    );
    expect(html).toContain('aria-label="Aktionen für die Meldung zu Filmclub"');
    expect(html).toContain('data-label="Gemeldet von"');
    expect(html).toContain('class="operator-details"');
  });

  it("shows how a closed report was closed", () => {
    const hourAgo = new Date(Date.now() - 60 * 60_000).toISOString();
    fixture.result = {
      data: page([
        report({
          status: "dismissed",
          resolved_at: hourAgo,
          resolved_by: { id: "lena", name: "Lena" },
          resolution_note: "Geklärt mit der Inhaberin",
        }),
        report({ id: "r2", status: "actioned", resolved_at: hourAgo }),
      ]),
    };
    const body = text(render());
    expect(body).toContain(
      "vor 5 Minuten Ohne Folgen Lena · vor 1 Stunde Geklärt mit der Inhaberin",
    );
    expect(body).toContain(
      "vor 5 Minuten Maßnahme Gelöschtes Konto · vor 1 Stunde",
    );
  });

  it("names every reason in both languages", () => {
    const reasons = ["spam", "harassment", "sexual", "illegal", "other"];
    fixture.result = {
      data: page(
        reasons.map((reason, index) =>
          report({ id: `r${index}`, reason: reason as Report["reason"] }),
        ),
      ),
    };
    const body = text(render());
    for (const label of [
      "Spam",
      "Belästigung",
      "Sexuelle Inhalte",
      "Rechtswidrige Inhalte",
      "Anderes",
    ])
      expect(body).toContain(label);
    setLocale("en", { persist: false });
    const english = text(render());
    for (const label of [
      "Harassment",
      "Sexual content",
      "Illegal content",
      "Other",
      "Reported by",
    ])
      expect(english).toContain(label);
  });

  it("has a load error", () => {
    fixture.result = { isError: true };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Die Meldungen konnten nicht geladen werden.");
  });
});
