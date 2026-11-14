import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { DirectoryStatus, Listing } from "../discover/types.ts";
import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  status: undefined as DirectoryStatus | undefined,
  listings: {} as {
    data?: { pages: { listings: unknown[]; next_offset: number | null }[] };
    isError?: boolean;
    hasNextPage?: boolean;
    isPlaceholderData?: boolean;
  },
  search: {} as { q?: string; lang?: string },
  queries: [] as [string, string | undefined][],
  enabled: [] as boolean[],
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor that says where it leads, without the router.
  Link: ({
    to,
    params,
    ...props
  }: {
    to: string;
    params?: Record<string, string>;
  } & Record<string, unknown>) => (
    <a
      data-to={to.replace(
        /\$(\w+)/g,
        (_, name: string) => params?.[name] ?? "",
      )}
      {...props}
    />
  ),
  useNavigate: () => vi.fn(),
  useSearch: () => fixture.search,
}));
vi.mock("../components/UserPanel.tsx", () => ({
  UserPanel: () => <footer data-user-panel="" />,
}));
vi.mock("../discover/queries.ts", () => ({
  useDirectoryStatus: () => ({ data: fixture.status }),
  useListings: (q: string, language: string | undefined, enabled = true) => {
    fixture.queries.push([q, language]);
    fixture.enabled.push(enabled);
    return { refetch: vi.fn(), fetchNextPage: vi.fn(), ...fixture.listings };
  },
  useJoinListing: () => ({ mutate: vi.fn(), isPending: false }),
  useRequestJoin: () => ({ mutate: vi.fn(), isPending: false }),
  useWithdrawRequest: () => ({ mutate: vi.fn(), isPending: false }),
  useReportListing: () => ({ mutate: vi.fn(), isPending: false }),
}));

import {
  ReportListingDialog,
  RequestJoinDialog,
} from "../components/DirectoryDialogs.tsx";
import { DiscoverPage, DiscoverRow, DiscoverSidebar } from "./DiscoverPage.tsx";

const LANGUAGES = [
  { code: "de", name: "Deutsch", listings: 3 },
  { code: "en", name: "English", listings: 1 },
  { code: "es", name: "Español", listings: 0 },
];

const listing = (overrides: Partial<Listing> = {}): Listing => ({
  server_id: "filmclub",
  name: "Filmclub",
  icon_url: null,
  description: "Jeden Freitag ein Film im Kino-Kanal, danach wird gestritten.",
  language: "de",
  member_count: 48,
  join_mode: "open",
  member: false,
  request: null,
  retry_at: null,
  ...overrides,
});
const pages = (listings: Listing[], next_offset: number | null = null) => ({
  pages: [{ listings, next_offset }],
});

beforeEach(() => {
  fixture.status = { enabled: true, languages: LANGUAGES };
  fixture.listings = {};
  fixture.search = {};
  fixture.queries = [];
  fixture.enabled = [];
});
afterEach(() => setLocale("de", { persist: false }));

const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
const row = (overrides: Partial<Listing> = {}) =>
  renderToStaticMarkup(
    <DiscoverRow
      listing={listing(overrides)}
      languages={LANGUAGES}
      onRequest={vi.fn()}
    />,
  );

describe("Entdecken", () => {
  it("shows the reference's title, line and search", () => {
    fixture.listings = { data: pages([]) };
    const html = renderToStaticMarkup(<DiscoverPage />);
    expect(html).toContain(">Entdecken</h1>");
    expect(html).toContain(
      "Server, die ihre Inhaber ins Verzeichnis gestellt haben. Alle anderen bleiben unsichtbar.",
    );
    expect(html).toContain('placeholder="Nach Name oder Thema suchen"');
    expect(html).toContain('maxLength="100"');
  });

  it("asks for the search and language in the address", () => {
    fixture.listings = { data: pages([]) };
    renderToStaticMarkup(<DiscoverPage q="rust" lang="en" />);
    expect(fixture.queries.at(-1)).toEqual(["rust", "en"]);
    expect(renderToStaticMarkup(<DiscoverPage q="rust" />)).toContain(
      'value="rust"',
    );
    expect(fixture.queries.at(-1)).toEqual(["rust", undefined]);
  });

  it("says when nothing was found", () => {
    fixture.listings = { data: pages([]) };
    expect(text(renderToStaticMarkup(<DiscoverPage />))).toContain(
      "Keine Server gefunden.",
    );
  });

  it("draws a row per server and offers more when there are", () => {
    fixture.listings = {
      data: pages(
        [
          listing(),
          listing({
            server_id: "rust",
            name: "Rust Study Hall",
            language: "en",
            member_count: 1,
          }),
        ],
        30,
      ),
      hasNextPage: true,
    };
    const html = renderToStaticMarkup(<DiscoverPage />);
    expect(html.match(/class="discover-row"/g)).toHaveLength(2);
    expect(text(html)).toContain("Deutsch · 48 Mitglieder");
    expect(text(html)).toContain("English · 1 Mitglied");
    expect(text(html)).toContain("Mehr laden");
    fixture.listings.hasNextPage = false;
    expect(text(renderToStaticMarkup(<DiscoverPage />))).not.toContain(
      "Mehr laden",
    );
  });

  it("offers no more while a new search shows the previous one's rows", () => {
    // keepPreviousData: the rows and next page are the previous search's.
    fixture.listings = {
      data: pages([listing()], 30),
      hasNextPage: true,
      isPlaceholderData: true,
    };
    const html = text(renderToStaticMarkup(<DiscoverPage q="rust" />));
    expect(html).toContain("Filmclub");
    expect(html).not.toContain("Mehr laden");
  });

  it("shows a server's image, else its initials", () => {
    expect(row()).toContain(">Fi</span>");
    expect(row({ icon_url: "/api/server-icons/filmclub/a.webp" })).toContain(
      'src="/api/server-icons/filmclub/a.webp"',
    );
  });

  it("offers the button that fits the caller", () => {
    const button = (overrides: Partial<Listing>) => {
      const html = row(overrides);
      const match = html.match(/<(button|a)([^>]*)>([^<]*)<\/\1>\s*<\/li>$/);
      return {
        label: match?.[3],
        primary: match?.[2].includes("ui-button--primary") ?? false,
        disabled: match?.[2].includes("disabled") ?? false,
        focusable: !/ disabled=""/.test(match?.[2] ?? ""),
        to: match?.[2].match(/data-to="([^"]+)"/)?.[1],
      };
    };
    expect(button({ member: true })).toEqual({
      label: "Öffnen",
      primary: false,
      disabled: false,
      focusable: true,
      to: "/s/filmclub",
    });
    expect(button({ join_mode: "open" })).toMatchObject({
      label: "Beitreten",
      primary: true,
      disabled: false,
    });
    expect(button({ join_mode: "request" })).toMatchObject({
      label: "Beitritt anfragen",
      primary: false,
      disabled: false,
    });
    expect(button({ join_mode: "request", request: "pending" })).toMatchObject({
      label: "Angefragt",
      disabled: true,
      focusable: false,
    });
    // Declined stays focusable, so its tooltip reaches the keyboard too.
    expect(button({ join_mode: "request", request: "declined" })).toMatchObject(
      { label: "Nicht angenommen", disabled: true, focusable: true },
    );
  });

  it("says from when a declined person may ask again", () => {
    const html = row({
      join_mode: "request",
      request: "declined",
      retry_at: "2026-11-18T09:30:00Z",
    });
    expect(html).toContain('aria-disabled="true"');
    expect(html).not.toContain(" title=");
    // Also for screen readers, next to the server's name.
    const id = html.match(/<h2 id="([^"]+)">Filmclub<\/h2>/)?.[1];
    expect(html).toContain(`aria-describedby="${id} ${id}-until"`);
    expect(html).toContain(
      `<span id="${id}-until" class="sr-only">Du kannst ab dem 18. November 2026 wieder anfragen.</span>`,
    );
  });

  it("shows a skeleton while loading and an error with a retry", () => {
    expect(renderToStaticMarkup(<DiscoverPage />)).toContain(
      "discover-skeleton",
    );
    fixture.listings = { isError: true };
    const failed = text(renderToStaticMarkup(<DiscoverPage />));
    expect(failed).toContain("Das Verzeichnis konnte nicht geladen werden.");
    expect(failed).toContain("Erneut versuchen");
  });

  it("keeps the rows and says so when more could not be loaded", () => {
    fixture.listings = {
      data: pages([listing()], 30),
      hasNextPage: true,
      isError: true,
    };
    const html = text(renderToStaticMarkup(<DiscoverPage />));
    expect(html).toContain("Filmclub");
    expect(html).toContain("Das Verzeichnis konnte nicht geladen werden.");
  });

  it("is not found once the directory was switched off", () => {
    fixture.status = { enabled: false, languages: [] };
    fixture.listings = { data: pages([listing()]) };
    const html = renderToStaticMarkup(<DiscoverPage />);
    expect(html).not.toContain("Filmclub");
    expect(html).not.toContain(">Entdecken</h1>");
    expect(fixture.enabled.at(-1)).toBe(false);
  });

  it("names the server for the button", () => {
    const html = row();
    const id = html.match(/<h2 id="([^"]+)">Filmclub<\/h2>/)?.[1];
    expect(id).toBeTruthy();
    expect(html).toContain(`aria-describedby="${id}"`);
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    fixture.listings = { data: pages([listing({ join_mode: "request" })]) };
    const html = text(renderToStaticMarkup(<DiscoverPage />));
    expect(html).toContain("Discover");
    expect(html).toContain("Deutsch · 48 members");
    expect(html).toContain("Request to join");
  });
});

describe("the language filter", () => {
  const buttons = (html: string) =>
    [
      ...html.matchAll(
        /<button[^>]*aria-pressed="(true|false)"[^>]*>([^<]+)</g,
      ),
    ].map(([, pressed, label]) => [label, pressed === "true"]);

  it("lists All and every language with listings", () => {
    const html = renderToStaticMarkup(<DiscoverSidebar />);
    expect(html).toContain('<div class="sidebar-brand">Entdecken</div>');
    expect(html).toContain(">Sprache</h2>");
    expect(buttons(html)).toEqual([
      ["Alle", true],
      ["Deutsch", false],
      ["English", false],
    ]);
    expect(html).toContain('lang="en"');
  });

  it("marks the language in the address", () => {
    fixture.search = { lang: "en" };
    expect(buttons(renderToStaticMarkup(<DiscoverSidebar />))).toEqual([
      ["Alle", false],
      ["Deutsch", false],
      ["English", true],
    ]);
  });
});

describe("the request dialog", () => {
  it("names the server and takes an optional message", () => {
    const html = renderToStaticMarkup(
      <RequestJoinDialog
        listing={listing({ name: "Modellbau-Keller", join_mode: "request" })}
        onClose={vi.fn()}
      />,
    );
    expect(text(html)).toContain("Beitritt anfragen");
    expect(text(html)).toContain("Anfrage an Modellbau-Keller");
    expect(text(html)).toContain("Nachricht (optional)");
    expect(html).toContain('maxLength="300"');
    expect(text(html)).toContain("Anfrage senden");
  });

  it("is closed without a server", () => {
    const html = renderToStaticMarkup(
      <RequestJoinDialog listing={null} onClose={vi.fn()} />,
    );
    expect(html).not.toContain("Anfrage senden");
  });
});

describe("the row menu", () => {
  it("is a named … button after the row's action", () => {
    const html = renderToStaticMarkup(
      <DiscoverRow
        listing={listing()}
        languages={LANGUAGES}
        onRequest={vi.fn()}
        onReport={vi.fn()}
      />,
    );
    expect(html).toContain('aria-label="Mehr zu Filmclub"');
    expect(html).toContain('aria-haspopup="menu"');
    expect(html).toContain('aria-expanded="false"');
    expect(html.indexOf("Beitreten")).toBeLessThan(
      html.indexOf("Mehr zu Filmclub"),
    );
  });

  it("is on every row of the page, and not in a row without reports", () => {
    fixture.listings = {
      data: pages([listing(), listing({ server_id: "b", name: "B" })]),
    };
    const html = renderToStaticMarkup(<DiscoverPage />);
    expect(html).toContain('aria-label="Mehr zu Filmclub"');
    expect(html).toContain('aria-label="Mehr zu B"');
    expect(row()).not.toContain("Mehr zu");
  });
});

describe("the report dialog", () => {
  const dialog = () =>
    renderToStaticMarkup(
      <ReportListingDialog listing={listing()} onClose={vi.fn()} />,
    );

  it("offers the five reasons, details and Melden", () => {
    const html = dialog();
    expect(text(html)).toContain("Server melden");
    expect(text(html)).toContain("Meldung zu Filmclub.");
    expect(
      [...html.matchAll(/type="radio"[^>]*value="(\w+)"/g)].map((m) => m[1]),
    ).toEqual(["spam", "harassment", "sexual", "illegal", "other"]);
    for (const label of [
      "Spam",
      "Belästigung",
      "Sexuelle Inhalte",
      "Rechtswidrige Inhalte",
      "Sonstiges",
    ])
      expect(text(html)).toContain(label);
    // No reason is chosen for the reporter.
    expect(html).not.toContain("checked");
    expect(html).toContain("<textarea");
    expect(html).toContain('maxLength="1000"');
    expect(text(html)).toContain("Bei „Sonstiges“ nötig.");
    expect(html).toMatch(/<button[^>]*type="submit"[^>]*>[^<]*Melden/);
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    const html = text(dialog());
    expect(html).toContain("Report server");
    expect(html).toContain("Harassment");
    expect(html).toContain("Other");
    expect(html).toContain("Report");
  });

  it("is closed without a server", () => {
    const html = renderToStaticMarkup(
      <ReportListingDialog listing={null} onClose={vi.fn()} />,
    );
    expect(html).not.toContain("Server melden");
  });
});
