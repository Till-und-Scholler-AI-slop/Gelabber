import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DirectMessage } from "../dms/types.ts";

const fixture = vi.hoisted(() => ({
  drawer: false,
  dms: [] as unknown[],
  rows: 0,
}));

vi.mock("@tanstack/react-query", () => ({ useQueryClient: () => ({}) }));
vi.mock("../auth/scope.ts", () => ({ useUserId: () => "u1" }));
vi.mock("../components/useMediaQuery.ts", () => ({
  useMediaQuery: (query: string) => {
    expect(query).toBe("(max-width: 800px)");
    return fixture.drawer;
  },
}));
vi.mock("../components/DmSidebar.tsx", () => ({
  DmRow: ({ dm }: { dm: DirectMessage }) => {
    fixture.rows++;
    return <a className="dm-row">{dm.peer.name}</a>;
  },
}));
vi.mock("../components/Redirect.tsx", () => ({ Redirect: () => null }));
vi.mock("../dms/lastDm.ts", () => ({
  lastDmId: () => null,
  useLastDm: (select: (state: unknown) => unknown) =>
    select({ byUser: {}, forget: () => {} }),
}));
vi.mock("../dms/queries.ts", () => ({
  forgetDm: () => {},
  useDm: () => ({ isError: false, error: null }),
  useDms: () => ({
    data: fixture.dms,
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: () => {},
  }),
}));

import { DmIndexPage } from "./DmIndexPage.tsx";

const dm = (id: string, name: string) => ({
  id,
  peer: { id: `p-${id}`, name },
});

beforeEach(() => {
  fixture.rows = 0;
  fixture.dms = [dm("d1", "Silas"), dm("d2", "Rafi")];
});

describe("DM index", () => {
  it("lists the conversations while the navigation is a drawer", () => {
    fixture.drawer = true;
    const html = renderToStaticMarkup(<DmIndexPage />);
    expect(html).toContain('<nav aria-label="Unterhaltungen"');
    expect(html).toContain("<h1>Nachrichten</h1>");
    expect(html).toContain(">Silas</a>");
    expect(html).toContain(">Rafi</a>");
    expect(html).not.toContain("Wähle eine Unterhaltung");
    expect(html).toContain("Neue Nachricht</button>");
  });

  it("mounts no rows beside a visible sidebar", () => {
    fixture.drawer = false;
    const html = renderToStaticMarkup(<DmIndexPage />);
    expect(html).toContain("Wähle eine Unterhaltung.");
    expect(html).toContain("Neue Nachricht</button>");
    expect(html).not.toContain("<nav");
    expect(fixture.rows).toBe(0);
  });

  it("starts a new message when there is no conversation yet", () => {
    fixture.drawer = true;
    fixture.dms = [];
    const html = renderToStaticMarkup(<DmIndexPage />);
    expect(html).toContain("<h1>Keine Unterhaltungen</h1>");
    expect(html).toContain("ui-button--primary");
    expect(html).toContain("Neue Nachricht</button>");
    expect(html).not.toContain("<nav");
  });
});
