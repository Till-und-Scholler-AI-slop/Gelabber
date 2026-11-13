import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../../i18n/locale.ts";
import type {
  FeedbackQuery,
  OperatorFeedback,
  OperatorFeedbackPage,
} from "../types.ts";

const fixture = vi.hoisted(() => ({
  result: {} as { data?: unknown; isError?: boolean },
  entry: {} as { data?: unknown; isError?: boolean; error?: unknown },
  entryIds: [] as (string | undefined)[],
  mail: true,
  queries: [] as unknown[],
  chips: [] as {
    label: string;
    value: string;
    options: { value: string; label: string }[];
  }[],
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  useNavigate: () => vi.fn(),
}));
vi.mock("../queries.ts", () => ({
  useOperatorFeedback: (query: FeedbackQuery) => {
    fixture.queries.push(query);
    return { refetch: vi.fn(), ...fixture.result };
  },
  useOperatorFeedbackEntry: (id: string | undefined) => {
    fixture.entryIds.push(id);
    return { refetch: vi.fn(), ...fixture.entry };
  },
  useOperatorOverview: () => ({
    data: {
      registration: "open",
      mail: fixture.mail,
      open_reports: 0,
      open_feedback: 1,
    },
  }),
  useOperatorRefresh: () => vi.fn(),
}));
vi.mock("../../components/ui/ChipGroup.tsx", () => ({
  ChipGroup: (props: {
    label: string;
    value: string;
    options: { value: string; label: string }[];
  }) => {
    fixture.chips.push(props);
    return null;
  },
}));

import { ApiError } from "../../api/client.ts";
import { FeedbackCard, FeedbackPage } from "./FeedbackPage.tsx";

const ago = (minutes: number) =>
  new Date(Date.now() - minutes * 60_000).toISOString();

const entry = (
  overrides: Partial<OperatorFeedback> = {},
): OperatorFeedback => ({
  id: "f1",
  kind: "problem",
  status: "open",
  created_at: ago(30),
  last_activity_at: ago(5),
  done_at: null,
  done_by: null,
  sender: {
    type: "account",
    id: "tom",
    name: "Tom",
    email: "tom@example.org",
    avatar_url: null,
  },
  text: "Beim Bildschirm teilen bleibt das Vorschaubild schwarz.",
  image: null,
  context: {
    client: "web",
    os: "linux",
    app_version: "0.10.0",
    locale: "de",
    page: "/s/home/c/sofa",
  },
  awaiting_reply: true,
  messages: [],
  ...overrides,
});
const page = (entries: OperatorFeedback[]) =>
  ({ entries, total: entries.length }) satisfies OperatorFeedbackPage;

beforeEach(() => {
  fixture.result = {};
  fixture.entry = {};
  fixture.entryIds = [];
  fixture.mail = true;
  fixture.queries = [];
  fixture.chips = [];
});
afterEach(() => setLocale("de", { persist: false }));

const text = (html: string) =>
  html
    .replace(/<[^>]+>/g, " ")
    .replace(/\s+/g, " ")
    .trim();
const card = (props: Partial<Parameters<typeof FeedbackCard>[0]> = {}) =>
  renderToStaticMarkup(
    <FeedbackCard
      entry={entry()}
      mail
      refresh={vi.fn()}
      defaultOpen
      {...props}
    />,
  );

describe("Feedback page", () => {
  it("asks for open entries of every kind, with Status and Art chips", () => {
    fixture.result = { data: page([]) };
    const html = renderToStaticMarkup(<FeedbackPage />);
    expect(html).toContain(">Feedback</h1>");
    expect(html).toContain("Ideen, Probleme und Lob von Konten und Besuchern.");
    expect(fixture.queries.at(-1)).toEqual({
      status: "open",
      kind: undefined,
      offset: undefined,
    });
    expect(fixture.chips.map(({ label, value }) => [label, value])).toEqual([
      ["Status", "open"],
      ["Art", "all"],
    ]);
    expect(fixture.chips[0]?.options.map((option) => option.label)).toEqual([
      "Offen",
      "Erledigt",
    ]);
    expect(fixture.chips[1]?.options).toEqual([
      { value: "all", label: "Alle" },
      { value: "idea", label: "Idee" },
      { value: "problem", label: "Problem" },
      { value: "praise", label: "Lob" },
    ]);
    expect(html).toContain("Kein offenes Feedback.");
    expect(fixture.entryIds).toEqual([]);
  });

  it("shows loading and a load error", () => {
    expect(renderToStaticMarkup(<FeedbackPage />)).toContain("ui-skeleton");
    fixture.result = { isError: true };
    expect(renderToStaticMarkup(<FeedbackPage />)).toContain(
      "Feedback konnte nicht geladen werden.",
    );
  });

  it("draws closed cards: kind, sender, awaiting reply, image, two lines", () => {
    fixture.result = {
      data: page([
        entry({
          image: {
            url: "/api/operator/feedback/f1/image",
            content_type: "image/png",
            size: 1200,
          },
        }),
        entry({
          id: "f2",
          kind: "idea",
          sender: { type: "visitor", email: "ada@example.org", locale: "en" },
          awaiting_reply: false,
        }),
        entry({ id: "f3", kind: "praise", sender: { type: "deleted" } }),
      ]),
    };
    const html = renderToStaticMarkup(<FeedbackPage />);
    const cards = html
      .split("<article")
      .slice(1)
      .map((part) => `<article${part}`);
    expect(cards).toHaveLength(3);
    expect(text(cards[0] ?? "")).toMatch(
      /^Problem Tom tom@example\.org Antwort offen Mit Bild vor 5 Minuten Beim Bildschirm/,
    );
    expect(text(cards[1] ?? "")).toMatch(/^Idee Besucher ada@example\.org vor/);
    expect(text(cards[2] ?? "")).toMatch(/^Lob Gelöschtes Konto Antwort offen/);
    // Closed: a header button, nothing behind it yet.
    expect(cards[0]).toMatch(
      /<button type="button" id="[^"]+-head" class="operator-feedback-head" aria-expanded="false" aria-labelledby="([^"]+-line)">/,
    );
    // Named by its line, not by the excerpt of up to 4,000 characters.
    const line = /aria-labelledby="([^"]+-line)"/.exec(cards[0] ?? "")?.[1];
    expect(cards[0]).toContain(
      `<span id="${line}" class="operator-feedback-line">`,
    );
    expect(cards[0]).toMatch(
      new RegExp(`^<article[^>]*aria-labelledby="${line}"`),
    );
    expect(html).not.toContain("operator-feedback-body");
    expect(html).toContain('class="operator-feedback-excerpt"');
  });

  it("speaks English after a switch", () => {
    setLocale("en", { persist: false });
    fixture.result = { data: page([]) };
    const html = renderToStaticMarkup(<FeedbackPage />);
    expect(html).toContain("No open feedback.");
    expect(fixture.chips[1]?.options.map((option) => option.label)).toEqual([
      "All",
      "Idea",
      "Problem",
      "Praise",
    ]);
  });

  it("opens the entry of ?entry= above the list", () => {
    fixture.result = { data: page([]) };
    fixture.entry = { data: entry({ id: "f9" }) };
    const html = renderToStaticMarkup(<FeedbackPage entry="f9" />);
    expect(fixture.entryIds).toEqual(["f9"]);
    expect(html).toMatch(
      /<section class="operator-feedback-mail" aria-labelledby="operator-feedback-mail"><h2 id="operator-feedback-mail" class="operator-feedback-mail-title">Aus der Mail<\/h2><article/,
    );
    expect(html).toContain('aria-expanded="true"');
    expect(html.indexOf("operator-feedback-mail")).toBeLessThan(
      html.indexOf("Kein offenes Feedback."),
    );
  });

  it("says when the entry of ?entry= is gone", () => {
    fixture.result = { data: page([]) };
    fixture.entry = {
      isError: true,
      error: new ApiError("not_found", 404, "gone"),
    };
    expect(renderToStaticMarkup(<FeedbackPage entry="f9" />)).toContain(
      "Dieses Feedback gibt es nicht mehr.",
    );
  });
});

describe("an open feedback card", () => {
  it("links the header button to the body it opens", () => {
    const html = card();
    const [, id] = html.match(/aria-controls="([^"]+)"/) ?? [];
    expect(id).toBeTruthy();
    expect(html).toContain(`<div id="${id}" class="operator-feedback-body">`);
    expect(html).toContain('aria-expanded="true"');
    expect(html).not.toContain("operator-feedback-excerpt");
  });

  it("shows the text, the image and the technical details", () => {
    const html = card({
      entry: entry({
        image: {
          url: "/api/operator/feedback/f1/image",
          content_type: "image/png",
          size: 1200,
        },
      }),
    });
    expect(html).toContain(
      '<p class="operator-feedback-text">Beim Bildschirm teilen bleibt das Vorschaubild schwarz.</p>',
    );
    expect(html).toMatch(
      /<button type="button" class="operator-feedback-image"><img src="\/api\/operator\/feedback\/f1\/image" alt="Angehängtes Bild"/,
    );
    expect(text(html)).toContain(
      "Technische Angaben App Web-App System Linux Version 0.10.0 Sprache Deutsch Seite /s/home/c/sofa",
    );
    expect(html).toContain("<code>/s/home/c/sofa</code>");
  });

  it("names the thread's authors and the mail states of visitor replies", () => {
    const lena = { id: "lena", name: "Lena" };
    const html = card({
      entry: entry({
        sender: { type: "visitor", email: "ada@example.org", locale: "de" },
        context: {
          client: "form",
          os: null,
          app_version: null,
          locale: "de",
          page: null,
        },
        messages: [
          ["m1", "sent"],
          ["m2", "pending"],
          ["m3", "failed"],
        ].map(([id, mail]) => ({
          id: id ?? "",
          from: "operator" as const,
          author: lena,
          text: `Antwort ${id}`,
          created_at: ago(1),
          mail: mail as "sent" | "pending" | "failed",
        })),
      }),
    });
    const thread = text(html.split("operator-feedback-thread")[1] ?? "");
    expect(thread).toMatch(/Lena .* Mail verschickt Antwort m1/);
    expect(thread).toMatch(/Lena .* Mail wird verschickt… Antwort m2/);
    expect(html).toContain(
      '<span class="ui-badge ui-badge--danger">Mail nicht zugestellt</span>',
    );
    expect(text(html)).toContain("App Formular auf der Website System");
  });

  it("puts the reply note per sender under the field", () => {
    expect(text(card())).toContain(
      "Antwort Tom sieht die Antwort unter Einstellungen → Feedback und bekommt eine Mail, dass es eine Antwort gibt.",
    );
    expect(text(card({ mail: false }))).toContain(
      "Tom sieht die Antwort unter Einstellungen → Feedback. Antworten",
    );
    const visitor = entry({
      sender: { type: "visitor", email: "ada@example.org", locale: "de" },
    });
    expect(text(card({ entry: visitor }))).toContain(
      "Geht per Mail an ada@example.org, mit deinem Text als Zitat.",
    );
    const off = card({ entry: visitor, mail: false });
    expect(off).not.toContain("<textarea");
    expect(off).toContain(
      '<p class="operator-notice">Ohne Account-Mail erreicht eine Antwort diese Person nicht.</p>',
    );
    // The note describes the field.
    expect(card()).toMatch(/<textarea[^>]*aria-describedby="[^"]+-hint"/);
    // Until GET /api/operator is in, neither the form nor the refusal.
    const unknown = card({ entry: visitor, mail: undefined });
    expect(unknown).not.toContain("<textarea");
    expect(unknown).not.toContain("Ohne Account-Mail");
  });

  it("offers Erledigt on an open entry and Wieder öffnen on a done one", () => {
    expect(text(card())).toMatch(/Erledigt$/);
    expect(card()).toContain('aria-label="Aktionen für Tom"');
    const done = card({
      entry: entry({ status: "done", done_at: ago(1) }),
    });
    expect(text(done)).toMatch(/Wieder öffnen$/);
  });
});
