import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { Member, ServerDetail } from "../servers/types.ts";
import { MemberPanel } from "./MemberPanel.tsx";

const SERVER = "server-1";
const at = "2026-11-17T00:00:00Z";

function member(n: number): Member {
  return {
    user_id: `user-${String(n).padStart(5, "0")}`,
    name: `Member ${String(n).padStart(5, "0")}`,
    avatar_url: null,
    joined_at: at,
    role: "member",
    role_ids: [],
    status_text: null,
    accent_color: null,
    avatar_decoration: null,
  } as Member;
}

function server(members: Member[]): ServerDetail {
  return {
    id: SERVER,
    name: "Load",
    owner_id: members[0]?.user_id ?? "",
    created_at: at,
    role: "member",
    permissions: [],
    member_permissions: [],
    categories: [],
    channels: [],
    members,
  } as unknown as ServerDetail;
}

function render(members: Member[]): string {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      <MemberPanel server={server(members)} forceVisible />
    </QueryClientProvider>,
  );
}

describe("member panel", () => {
  it("mounts only the rows in view, not every member (#613)", () => {
    const members = Array.from({ length: 2000 }, (_, n) => member(n));
    const html = render(members);
    const mounted = html.match(/role="listitem"/g)?.length ?? 0;
    expect(mounted).toBeGreaterThan(5);
    expect(mounted).toBeLessThan(60);
    // The whole list is still announced, as members: headings not counted.
    expect(html).toContain('aria-setsize="2000"');
    expect(html).toContain('aria-posinset="1"');
    expect(html).not.toContain('aria-setsize="2001"');
    expect(html).toContain("Member 00000");
    expect(html).not.toContain("Member 01999");
  });

  it("gives the list one Tab stop and the key hint on its first member", () => {
    const html = render(Array.from({ length: 30 }, (_, n) => member(n)));
    expect(html.match(/tabindex="0"/g)?.length).toBe(1);
    const first = /<button[^>]*tabindex="0"[^>]*>/.exec(html)?.[0] ?? "";
    expect(first).toContain('aria-label="Profil von Member 00000"');
    expect(first).toMatch(/aria-describedby="[^"]+"/);
    expect(html.match(/aria-describedby=/g)?.length).toBe(1);
  });

  it("heads a presence group with its count and fades offline rows", () => {
    // Nobody has presence yet: everyone is offline.
    const html = render([member(1), member(2), member(3)]);
    const headings = [...html.matchAll(/<h3[^>]*>([^<]*)<\/h3>/g)].map(
      (match) => match[1],
    );
    expect(headings).toEqual(["Offline — 3"]);
    expect(html.match(/member-row is-offline/g)?.length).toBe(3);
  });
});
