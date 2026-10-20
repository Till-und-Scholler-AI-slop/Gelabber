import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Member, Server } from "../servers/types.ts";

const fixture = vi.hoisted(() => ({
  kickPending: false,
  banPending: false,
}));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "self", name: "Rafi", avatar_url: null } }),
}));
vi.mock("../servers/queries.ts", () => ({
  useKickMember: () => ({ isPending: fixture.kickPending, mutate: vi.fn() }),
  useBanMember: () => ({ isPending: fixture.banPending, mutate: vi.fn() }),
}));

import { MemberActions } from "./MemberActions.tsx";

const server: Pick<Server, "id" | "role" | "permissions"> = {
  id: "s",
  role: "member",
  permissions: ["manage_server"],
};
const member = (overrides: Partial<Member> = {}): Member => ({
  user_id: "jonas",
  name: "Jonas",
  avatar_url: null,
  joined_at: "2026-01-01T00:00:00Z",
  role: "member",
  ...overrides,
});
/** The attribute React writes for a disabled button. */
const DISABLED = ' disabled=""';
/** The opening tag of the button whose accessible name is `label`. */
function button(html: string, label: string): string {
  const match = html.match(
    new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`),
  );
  expect(match).not.toBeNull();
  return match![0];
}

describe("member actions", () => {
  beforeEach(() => {
    fixture.kickPending = false;
    fixture.banPending = false;
  });

  it("names each button after the member it acts on", () => {
    const html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(button(html, "Kick Jonas")).not.toContain(DISABLED);
    expect(button(html, "Ban Jonas")).not.toContain(DISABLED);
    expect(html).toContain(">Kick</button>");
    expect(html).toContain(">Ban</button>");
  });

  it("allows one kick or ban at a time", () => {
    fixture.kickPending = true;
    let html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(button(html, "Kick Jonas")).toContain(DISABLED);
    expect(button(html, "Ban Jonas")).toContain(DISABLED);

    fixture.kickPending = false;
    fixture.banPending = true;
    html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(button(html, "Kick Jonas")).toContain(DISABLED);
    expect(button(html, "Ban Jonas")).toContain(DISABLED);
  });

  it("shows nothing for the owner, oneself or without manage_server", () => {
    const render = (s: typeof server, m: Member) =>
      renderToStaticMarkup(<MemberActions server={s} member={m} />);
    expect(render(server, member({ role: "owner" }))).toBe("");
    expect(render(server, member({ user_id: "self" }))).toBe("");
    expect(render({ ...server, permissions: [] }, member())).toBe("");
  });
});
