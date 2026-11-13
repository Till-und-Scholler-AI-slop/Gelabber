import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { FeedbackEntry, FeedbackPage } from "./types.ts";

const fixture = vi.hoisted(() => ({
  list: {} as Record<string, unknown>,
  refetch: vi.fn(),
  fetchNextPage: vi.fn(),
}));

vi.mock("./queries.ts", () => ({
  useFeedbackList: () => fixture.list,
  useReplyFeedback: () => ({ isPending: false, mutate: vi.fn() }),
  markFeedbackRead: vi.fn(() => Promise.resolve()),
}));

import { setLocale } from "../i18n/locale.ts";
import { FeedbackArea } from "./FeedbackArea.tsx";
import { entriesToRead, rememberNew, sentAlongLine } from "./thread.ts";

const entry = (overrides: Partial<FeedbackEntry> = {}): FeedbackEntry => ({
  id: "e1",
  kind: "problem",
  status: "open",
  text: "Das Vorschaubild\nbleibt schwarz.",
  created_at: "2026-11-03T10:00:00Z",
  last_activity_at: "2026-11-04T10:00:00Z",
  image: null,
  context: {
    client: "web",
    os: "linux",
    app_version: "0.10.0",
    locale: "de",
    page: "/s/home/c/sofa",
  },
  unread: false,
  messages: [],
  ...overrides,
});

const loaded = (entries: FeedbackEntry[], more = false) => {
  const page: FeedbackPage = { entries, total: entries.length };
  fixture.list = {
    isPending: false,
    isError: false,
    isFetching: false,
    data: { pages: [page], pageParams: [0] },
    hasNextPage: more,
    isFetchingNextPage: false,
    refetch: fixture.refetch,
    fetchNextPage: fixture.fetchNextPage,
  };
};

const render = () =>
  renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <FeedbackArea />
    </QueryClientProvider>,
  );

beforeEach(() => {
  loaded([]);
});

describe("Einstellungen → Feedback", () => {
  it("offers Feedback geben in every state", () => {
    fixture.list = { isPending: true, isError: false };
    const loading = render();
    expect(loading).toContain("ui-skeleton");
    expect(loading.match(/ui-skeleton-row"/g)).toHaveLength(3);
    expect(loading).toContain(">Feedback geben</button>");
  });

  it("says when the list could not be loaded, with a retry", () => {
    fixture.list = {
      isPending: false,
      isError: true,
      isFetching: false,
      data: undefined,
      refetch: fixture.refetch,
    };
    const html = render();
    expect(html).toContain('role="alert"');
    expect(html).toContain("Feedback konnte nicht geladen werden.");
    expect(html).toContain("Erneut versuchen");
  });

  it("explains an empty list", () => {
    const html = render();
    expect(html).toContain("Du hast noch kein Feedback geschickt.");
    expect(html).toContain(
      "Ideen, Probleme und Lob gehen direkt an die Betreiber dieser Instanz.",
    );
  });

  it("shows each entry as a thread with Du and Betreiber", () => {
    loaded([
      entry({
        messages: [
          {
            id: "m1",
            from: "operators",
            text: "Welchen Browser nutzt du?",
            created_at: "2026-11-03T11:00:00Z",
            new: false,
          },
          {
            id: "m2",
            from: "you",
            text: "Firefox.",
            created_at: "2026-11-03T12:00:00Z",
            new: false,
          },
        ],
      }),
    ]);
    const html = render();
    expect(html).toContain('<article class="feedback-entry"');
    expect(html).toContain('aria-labelledby="feedback-entry-e1"');
    expect(html).toMatch(/<h3 id="feedback-entry-e1"[^>]*>.*Problem.*Offen/);
    expect(html).toContain("3. November 2026");
    expect(html).toContain("Das Vorschaubild\nbleibt schwarz.");
    expect(html).toContain("<summary>Mitgeschickt</summary>");
    expect(html).toContain(
      "Web-App · Linux · 0.10.0 · Deutsch · /s/home/c/sofa",
    );
    expect(html).toMatch(/<strong>Betreiber<\/strong>.*Welchen Browser/s);
    expect(html).toMatch(/<strong>Du<\/strong>.*Firefox\./s);
    expect(html).not.toContain(">Neu<");
    expect(html).not.toContain("als erledigt markiert");
    expect(html).toContain(">Antworten</button>");
  });

  it("marks replies that were new, and notes a done entry", () => {
    loaded([
      entry({
        status: "done",
        unread: true,
        messages: [
          {
            id: "m1",
            from: "operators",
            text: "Behoben.",
            created_at: "2026-11-03T11:00:00Z",
            new: true,
          },
        ],
      }),
    ]);
    const html = render();
    expect(html).toContain('ui-badge--success">Erledigt</span>');
    expect(html).toContain('ui-badge--accent">Neu</span>');
    expect(html).toContain(
      "Die Betreiber haben das als erledigt markiert. Wenn du antwortest, ist es wieder offen.",
    );
  });

  it("shows the image as a button that enlarges it", () => {
    loaded([
      entry({
        image: {
          url: "/api/feedback/e1/image",
          content_type: "image/png",
          size: 10,
        },
      }),
    ]);
    expect(render()).toMatch(
      /<button type="button" class="feedback-entry__image"><img src="\/api\/feedback\/e1\/image" alt="Angehängtes Bild"/,
    );
  });

  it("loads more while entries are left", () => {
    loaded([entry()], true);
    expect(render()).toContain(">Mehr laden</button>");
    loaded([entry()], false);
    expect(render()).not.toContain("Mehr laden");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    try {
      loaded([
        entry({
          context: {
            client: "desktop",
            os: "other",
            app_version: "0.10.0",
            locale: "en",
            page: "/settings/darstellung",
          },
          messages: [
            {
              id: "m1",
              from: "operators",
              text: "Thanks!",
              created_at: "2026-11-03T11:00:00Z",
              new: true,
            },
          ],
        }),
      ]);
      const html = render();
      expect(html).toContain(">Give feedback</button>");
      expect(html).toContain("<strong>Operators</strong>");
      expect(html).toContain(">New</span>");
      expect(html).toContain(
        "Desktop app · Unknown · 0.10.0 · English · /settings/darstellung",
      );
    } finally {
      setLocale("de", { persist: false });
    }
  });
});

describe("the visit's read state", () => {
  it("marks read only the entries with unread replies, once, through the newest message", () => {
    const reply = (id: string) => ({
      id,
      from: "operators" as const,
      text: "Danke",
      created_at: "2026-11-03T11:00:00Z",
      new: true,
    });
    const entries = [
      entry({ id: "a", unread: true, messages: [reply("a1"), reply("a2")] }),
      entry({ id: "b" }),
      entry({ id: "c", unread: true, messages: [reply("c1")] }),
    ];
    expect(entriesToRead(entries, new Set())).toEqual([
      { id: "a", through: "a2" },
      { id: "c", through: "c1" },
    ]);
    expect(entriesToRead(entries, new Set(["a"]))).toEqual([
      { id: "c", through: "c1" },
    ]);
  });

  it("keeps the new replies it saw when a refetch says they are read", () => {
    const message = {
      id: "m1",
      from: "operators" as const,
      text: "Hallo",
      created_at: "2026-11-03T11:00:00Z",
    };
    const first = rememberNew(new Set(), [
      entry({ messages: [{ ...message, new: true }] }),
    ]);
    expect([...first]).toEqual(["m1"]);
    const after = rememberNew(first, [
      entry({ messages: [{ ...message, new: false }] }),
    ]);
    // The same set: nothing to store, and the mark stays.
    expect(after).toBe(first);
    expect(after.has("m1")).toBe(true);
  });

  it("leaves out what was not sent along", () => {
    expect(
      sentAlongLine({
        client: "web",
        os: null,
        app_version: null,
        locale: "de",
        page: null,
      }),
    ).toBe("Web-App · Deutsch");
  });
});
