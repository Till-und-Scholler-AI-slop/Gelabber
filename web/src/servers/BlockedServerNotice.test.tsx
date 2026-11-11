import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import type { Server } from "./types.ts";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => () => Promise.resolve(),
}));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "me", name: "Lena" } }),
}));

import { BlockedServerNotice } from "./BlockedServerNotice.tsx";

const server = (role: Server["role"]): Server => ({
  id: "s1",
  name: "Abendrunde",
  owner_id: role === "owner" ? "me" : "owner",
  created_at: "2026-01-01T00:00:00Z",
  role,
  permissions: [],
  member_permissions: [],
  blocked: true,
});

function render(listed: Server | undefined): string {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <BlockedServerNotice serverId="s1" server={listed} />
    </QueryClientProvider>,
  );
}

describe("blocked server notice", () => {
  afterEach(() => setLocale("de", { persist: false }));

  it("says the operator blocked it, and lets a member leave", () => {
    const html = render(server("member"));
    expect(html).toContain("<h1>Abendrunde</h1>");
    expect(html).toContain(
      '<p class="auth-lead">Dieser Server wurde vom Betreiber gesperrt.</p>',
    );
    expect(html).toMatch(/<button[^>]*>Server verlassen<\/button>/);
    expect(html).not.toContain("Server löschen");
  });

  it("lets the owner delete it", () => {
    const html = render(server("owner"));
    expect(html).toMatch(/<button[^>]*>Server löschen<\/button>/);
    expect(html).not.toContain("Server verlassen");
  });

  it("offers nothing while it cannot tell owner from member", () => {
    const html = render(undefined);
    expect(html).toContain(
      "<h1>Dieser Server wurde vom Betreiber gesperrt.</h1>",
    );
    expect(html).not.toContain("<button");
  });

  it("speaks English", () => {
    setLocale("en", { persist: false });
    expect(render(server("member"))).toContain(
      "This server has been blocked by the operator.",
    );
    expect(render(server("member"))).toMatch(/>Leave server<\/button>/);
    expect(render(server("owner"))).toMatch(/>Delete server<\/button>/);
  });
});
