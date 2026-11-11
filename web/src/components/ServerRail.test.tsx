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
    title,
    className,
    "aria-label": label,
    "aria-current": current,
    children,
  }: {
    to: string;
    title?: string;
    className?: string;
    "aria-label"?: string;
    "aria-current"?: "page";
    children?: ReactNode;
  }) => (
    <a
      data-to={to}
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

import { DiscoverTile } from "./ServerRail.tsx";

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
