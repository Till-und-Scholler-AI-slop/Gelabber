import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";

import type { ServerDetail } from "../servers/types.ts";

const fixture = vi.hoisted(() => ({
  server: null as unknown as ServerDetail,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="#">{children}</a>,
  useNavigate: () => vi.fn(),
  useParams: () => ({ serverId: "s1" }),
}));
vi.mock("../servers/queries.ts", () => {
  const mutation = () => ({ isPending: false, mutate: vi.fn() });
  return {
    readServer: vi.fn(),
    useServer: () => ({ data: fixture.server, error: null }),
    useBans: () => ({ data: [] }),
    useInvites: () => ({ data: [] }),
    useDeleteServer: mutation,
    useLeaveServer: mutation,
    useRevokeInvite: mutation,
    useServerIcon: mutation,
    useUnbanMember: mutation,
    useUpdateServer: mutation,
  };
});

import { ServerSettingsPage } from "./ServerSettingsPage.tsx";

function page(permissions: ServerDetail["permissions"]) {
  fixture.server = {
    id: "s1",
    name: "Wohnzimmer",
    owner_id: "u2",
    created_at: "2026-09-13T00:00:00Z",
    role: "member",
    permissions,
    member_permissions: [],
    categories: [],
    channels: [],
    members: [],
    roles: [],
  };
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <ServerSettingsPage />
    </QueryClientProvider>,
  );
}

const marked = (html: string) =>
  [...html.matchAll(/<a [^>]*aria-current="location"[^>]*>([^<]*)</g)].map(
    (match) => match[1],
  );

describe("server settings page", () => {
  it("marks the first section in the anchor row, and only that one", () => {
    expect(marked(page(["manage_server"]))).toEqual(["Allgemein"]);
    // Members without manage_server start at Mitglieder.
    expect(marked(page([]))).toEqual(["Mitglieder"]);
  });

  it("says what applies at once and what waits for Speichern", () => {
    const html = page(["manage_server"]);
    expect(html).toContain(
      "Ein neues Bild gilt sofort, den Namen übernimmst du mit Speichern.",
    );
    expect(html).toContain("Änderungen wirken sofort.");
  });

  it("tells why Speichern waits while the name is unchanged", () => {
    const html = page(["manage_server"]);
    const hint = html.match(/<span id="([^"]+)"[^>]*>Keine Änderungen\.</);
    expect(hint).not.toBeNull();
    expect(html).toMatch(
      new RegExp(`<button[^>]*aria-describedby="${hint?.[1]}"[^>]*>Speichern`),
    );
  });
});

describe("server settings page in English", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("labels the sections and the permissions in English", () => {
    setLocale("en", { persist: false });
    const html = page(["manage_server"]);
    expect(marked(html)).toEqual(["General"]);
    expect(html).toContain("Server settings");
    expect(html).toContain("Member permissions");
    expect(html).toContain("Go Live");
    expect(html).toContain("Danger zone");
    expect(html).not.toContain("Servereinstellungen");
  });
});
