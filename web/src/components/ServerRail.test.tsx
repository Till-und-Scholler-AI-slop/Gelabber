import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";

const fixture = vi.hoisted(() => ({
  status: undefined as { enabled: boolean } | undefined,
  pathname: "/",
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor that says where it leads, without the router.
  Link: ({
    to,
    params,
    title,
    className,
    "aria-label": label,
    "aria-current": current,
    children,
  }: {
    to: string;
    params?: Record<string, string>;
    title?: string;
    className?: string;
    "aria-label"?: string;
    "aria-current"?: "page";
    children?: ReactNode;
  }) => (
    <a
      data-to={to}
      data-params={params ? JSON.stringify(params) : undefined}
      title={title}
      className={className}
      aria-label={label}
      aria-current={current}
    >
      {children}
    </a>
  ),
  useRouterState: ({
    select,
  }: {
    select: (state: { location: { pathname: string } }) => unknown;
  }) => select({ location: { pathname: fixture.pathname } }),
}));
vi.mock("./navigationDrawer.ts", () => ({
  useNavigationDrawer: () => ({ open: false, keepOpen: undefined }),
}));
vi.mock("../discover/queries.ts", () => ({
  useDirectoryStatus: () => ({ data: fixture.status }),
}));
// The count itself is UnreadBadge's (its own tests); here only where it is.
vi.mock("../messages/UnreadBadge.tsx", () => ({
  UnreadBadge: ({ id, channelId }: { id?: string; channelId?: string }) => (
    <span id={id} data-badge={channelId} />
  ),
}));

import type { DirectMessage } from "../dms/types.ts";
import { DiscoverTile, UnreadDms } from "./ServerRail.tsx";

beforeEach(() => {
  fixture.status = undefined;
  fixture.pathname = "/";
});
afterEach(() => setLocale("de", { persist: false }));

const render = () => renderToStaticMarkup(<DiscoverTile />);

describe("the rail's Entdecken tile", () => {
  it("is there only while the directory is on", () => {
    expect(render()).toBe("");
    fixture.status = { enabled: false };
    expect(render()).toBe("");
    fixture.status = { enabled: true };
    const html = render();
    expect(html).toContain('data-to="/discover"');
    expect(html).toContain('aria-label="Entdecken"');
    expect(html).toContain('title="Entdecken"');
    expect(html).not.toContain("is-active");
  });

  it("is marked while the directory is open", () => {
    fixture.status = { enabled: true };
    fixture.pathname = "/discover";
    const html = render();
    expect(html).toContain('aria-current="page"');
    expect(html).toContain("server-discover-tile is-active");
  });

  it("speaks English", () => {
    fixture.status = { enabled: true };
    setLocale("en", { persist: false });
    expect(render()).toContain('aria-label="Discover"');
  });
});

const dm = (id: string, name: string, avatar: string | null) =>
  ({
    id,
    kind: "dm",
    created_at: "2026-11-23T00:00:00Z",
    peer: { id: `peer-${id}`, name, avatar_url: avatar },
  }) satisfies DirectMessage;

describe("the rail's unread direct messages", () => {
  it("are not there without any", () => {
    expect(renderToStaticMarkup(<UnreadDms dms={[]} />)).toBe("");
  });

  it("link each conversation with its avatar, count and a line under them", () => {
    const html = renderToStaticMarkup(
      <UnreadDms
        dms={[
          { dm: dm("d1", "Silas", "https://img.example/silas.png"), count: 2 },
          { dm: dm("d2", "Graf Scholler", null), count: 1 },
        ]}
      />,
    );
    expect(html).toContain('aria-label="Ungelesene Direktnachrichten"');
    expect(html).toContain('data-to="/d/$channelId"');
    expect(html).toContain(
      'data-params="{&quot;channelId&quot;:&quot;d1&quot;}"',
    );
    expect(html).toContain('aria-label="Silas"');
    expect(html).toContain('src="https://img.example/silas.png"');
    // A picture-less person shows their initials.
    expect(html).toContain(">GS</span>");
    expect(html).toContain('data-badge="d2"');
    expect(html.indexOf("Silas")).toBeLessThan(html.indexOf("Graf Scholler"));
    expect(html).toContain("server-dm-tile is-unread");
    expect(html).toContain('class="server-rail-separator"');
  });

  it("speak English", () => {
    setLocale("en", { persist: false });
    const html = renderToStaticMarkup(
      <UnreadDms dms={[{ dm: dm("d1", "Silas", null), count: 1 }]} />,
    );
    expect(html).toContain('aria-label="Unread direct messages"');
  });
});
