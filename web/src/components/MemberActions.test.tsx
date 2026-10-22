import type { ReactElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Member, Server } from "../servers/types.ts";

const fixture = vi.hoisted(() => ({
  kickPending: false,
  banPending: false,
  kick: vi.fn(),
  ban: vi.fn(),
  confirm: vi.fn<(options: { title: string }) => Promise<boolean>>(
    async () => false,
  ),
}));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "self", name: "Rafi", avatar_url: null } }),
}));
vi.mock("../servers/queries.ts", () => ({
  useKickMember: () => ({
    isPending: fixture.kickPending,
    mutate: fixture.kick,
  }),
  useBanMember: () => ({ isPending: fixture.banPending, mutate: fixture.ban }),
}));
vi.mock("./confirm.ts", () => ({ confirmAction: fixture.confirm }));

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
/** The opening tag of the button labelled `label`. */
function button(html: string, label: string): string {
  const match = html.match(new RegExp(`<button[^>]*>${label}</button>`));
  expect(match).not.toBeNull();
  return match![0];
}
type Props = { children?: unknown; onClick?: () => Promise<void> };
/** The kick and ban buttons, from the element tree MemberActions returns. */
function handlers() {
  const details = MemberActions({ server, member: member() }) as ReactElement<{
    children: ReactElement<Props>[];
  }>;
  const actions = details.props.children[1] as ReactElement<Props>;
  const [kick, ban] = actions.props.children as ReactElement<Props>[];
  return { kick: kick.props.onClick!, ban: ban.props.onClick! };
}

describe("member actions", () => {
  beforeEach(() => {
    fixture.kickPending = false;
    fixture.banPending = false;
    fixture.kick.mockReset();
    fixture.ban.mockReset();
    fixture.confirm.mockReset().mockResolvedValue(false);
  });

  it("keeps kick and ban in a closed Moderation section", () => {
    const html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(html).toMatch(
      /^<details class="member-moderation"><summary>Moderation<svg/,
    );
    expect(html).not.toContain("<details open");
    expect(button(html, "Vom Server entfernen")).not.toContain(DISABLED);
    expect(button(html, "Sperren")).not.toContain(DISABLED);
  });

  it("asks before it kicks or bans, and does nothing on Abbrechen", async () => {
    const { kick, ban } = handlers();
    await kick();
    await ban();
    expect(fixture.confirm).toHaveBeenCalledTimes(2);
    expect(fixture.confirm.mock.calls[0][0]).toMatchObject({
      title: "Jonas vom Server entfernen?",
      confirmLabel: "Entfernen",
    });
    expect(fixture.confirm.mock.calls[1][0]).toMatchObject({
      title: "Jonas sperren?",
      confirmLabel: "Sperren",
    });
    expect(fixture.kick).not.toHaveBeenCalled();
    expect(fixture.ban).not.toHaveBeenCalled();

    fixture.confirm.mockResolvedValue(true);
    await handlers().kick();
    expect(fixture.kick).toHaveBeenCalledWith("jonas");
    await handlers().ban();
    expect(fixture.ban).toHaveBeenCalledWith("jonas");
  });

  it("allows one kick or ban at a time", () => {
    fixture.kickPending = true;
    let html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(button(html, "Vom Server entfernen")).toContain(DISABLED);
    expect(button(html, "Sperren")).toContain(DISABLED);

    fixture.kickPending = false;
    fixture.banPending = true;
    html = renderToStaticMarkup(
      <MemberActions server={server} member={member()} />,
    );
    expect(button(html, "Vom Server entfernen")).toContain(DISABLED);
    expect(button(html, "Sperren")).toContain(DISABLED);
  });

  it("shows nothing for the owner, oneself or without manage_server", () => {
    const render = (s: typeof server, m: Member) =>
      renderToStaticMarkup(<MemberActions server={s} member={m} />);
    expect(render(server, member({ role: "owner" }))).toBe("");
    expect(render(server, member({ user_id: "self" }))).toBe("");
    expect(render({ ...server, permissions: [] }, member())).toBe("");
  });
});
