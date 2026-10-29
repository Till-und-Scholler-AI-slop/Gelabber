import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";

const fixture = vi.hoisted(() => ({
  preview: {} as Record<string, unknown>,
  join: { error: null as unknown, isPending: false, mutate: () => {} },
}));

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ code: "k7xm2pqrstuv" }),
  useNavigate: () => () => Promise.resolve(),
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@tanstack/react-query", () => ({
  useQuery: () => fixture.preview,
}));
vi.mock("../auth/scope.ts", () => ({
  useUserId: () => "u1",
  scopeGeneration: () => 1,
}));
vi.mock("../servers/queries.ts", () => ({
  serverKeys: { invite: () => ["invite"] },
  useJoinInvite: () => fixture.join,
}));

import { InvitePage } from "./InvitePage.tsx";

const loaded = (overrides: Record<string, unknown> = {}) => ({
  isPending: false,
  isError: false,
  isFetching: false,
  error: null,
  refetch: vi.fn(),
  data: {
    code: "k7xm2pqrstuv",
    server: { id: "s1", name: "Spieleabend", member_count: 3 },
    expires_at: null,
    member: false,
    ...overrides,
  },
});

const failed = (code: ConstructorParameters<typeof ApiError>[0]) => ({
  isPending: false,
  isError: true,
  isFetching: false,
  error: new ApiError(code, 500, code),
  refetch: vi.fn(),
  data: undefined,
});

beforeEach(() => {
  fixture.join = { error: null, isPending: false, mutate: () => {} };
});

describe("invite page", () => {
  it("shows the server and one way in, plus a way out", () => {
    fixture.preview = loaded();
    const html = renderToStaticMarkup(<InvitePage />);
    expect(html).toContain('<span aria-hidden="true" class="invite-tile"');
    expect(html).toContain(">S</span>");
    expect(html).toContain('<h1 id="invite-server-name">Spieleabend</h1>');
    expect(html).toContain("<p>3 Mitglieder</p>");
    expect(html).toMatch(/ui-button--primary[^>]*>Beitreten<\/button>/);
    expect(html).toContain('<a href="/">Abbrechen</a>');
  });

  it("says when the link runs out", () => {
    const inSixDays = new Date(Date.now() + 6 * 86_400_000 + 60_000);
    fixture.preview = loaded({ expires_at: inSixDays.toISOString() });
    expect(renderToStaticMarkup(<InvitePage />)).toContain(
      "<p>3 Mitglieder · läuft in 6 Tagen ab</p>",
    );
  });

  it("loads with a placeholder instead of a blank page", () => {
    fixture.preview = { isPending: true };
    const html = renderToStaticMarkup(<InvitePage />);
    expect(html).toContain("invite-skeleton");
    expect(html).toContain("Wird geladen…");
  });

  it("offers a retry when the preview could not be loaded", () => {
    for (const code of [
      "network",
      "timeout",
      "internal",
      "rate_limited",
    ] as const) {
      fixture.preview = failed(code);
      const html = renderToStaticMarkup(<InvitePage />);
      expect(html).toContain("Einladung konnte nicht geladen werden");
      expect(html).toContain(">Erneut versuchen</button>");
      expect(html).not.toContain("Einladung nicht nutzbar");
    }
  });

  it("asks for a new link when this one is dead", () => {
    for (const code of ["not_found", "invite_invalid"] as const) {
      fixture.preview = failed(code);
      const html = renderToStaticMarkup(<InvitePage />);
      expect(html).toContain("Einladung nicht nutzbar");
      expect(html).toContain("Frag nach einem neuen Link.");
      expect(html).not.toContain("Erneut versuchen");
    }
    fixture.preview = failed("banned");
    expect(renderToStaticMarkup(<InvitePage />)).toContain("Kein Zutritt");
  });

  it("turns a join that finds the link used up into that state", () => {
    fixture.preview = loaded();
    fixture.join = {
      error: new ApiError("invite_invalid", 410, "gone"),
      isPending: false,
      mutate: () => {},
    };
    const html = renderToStaticMarkup(<InvitePage />);
    expect(html).toContain("Einladung nicht nutzbar");
    expect(html).not.toContain(">Beitreten</button>");
  });
});
