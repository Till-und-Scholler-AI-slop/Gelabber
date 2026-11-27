import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";

import type { Badge } from "../messages/notifyLevel.ts";
import { sidebarRows } from "../servers/rows.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";

const fixture = vi.hoisted(() => ({
  badges: {} as Record<string, Badge>,
  drafts: [] as string[],
  speaking: [] as string[],
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain link with the row's attributes, without the router.
  Link: (props: Record<string, unknown>) => (
    <a
      href="#channel"
      {...Object.fromEntries(
        Object.entries(props).filter(
          ([name]) => name !== "to" && name !== "params",
        ),
      )}
    />
  ),
  useNavigate: () => () => Promise.resolve(),
}));
vi.mock("../messages/useBadge.ts", () => ({
  useBadge: ({ channelId }: { channelId?: string }) =>
    fixture.badges[channelId ?? ""] ?? { count: 0, dot: false },
  useBadges: () => new Map(),
}));
vi.mock("../voice/speaking.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../voice/speaking.ts")>()),
  useSpeaking: (_server: string, _channel: string, userId: string) =>
    fixture.speaking.includes(userId),
}));
vi.mock("../messages/drafts.ts", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../messages/drafts.ts")>()),
  useHasDraft: (_userId: string | null, channelId: string) =>
    fixture.drafts.includes(channelId),
}));

import {
  ChannelList,
  ChannelRow,
  ParticipantRowView,
} from "./ChannelSidebar.tsx";

beforeEach(() => {
  fixture.badges = {};
  fixture.drafts = [];
  fixture.speaking = [];
});

const created_at = "2026-09-13T00:00:00Z";
const channel = (
  id: string,
  category_id: string | null,
  kind: Channel["kind"] = "text",
): Channel => ({
  id,
  server_id: "s1",
  category_id,
  name: id,
  kind,
  created_at,
  limited: false,
  role_ids: [],
});
const server: ServerDetail = {
  id: "s1",
  name: "Spieleabend",
  owner_id: "u1",
  created_at,
  role: "member",
  permissions: [],
  member_permissions: [],
  blocked: false,
  categories: [{ id: "cat", server_id: "s1", name: "Planung", created_at }],
  // In the list: allgemein, "Sprachkanäle", wohnzimmer, "Planung", termine.
  channels: [
    channel("allgemein", null),
    channel("wohnzimmer", null, "voice"),
    channel("termine", "cat"),
  ],
  members: [],
  roles: [],
};

const wrap = (node: ReactNode) =>
  render(
    <QueryClientProvider client={new QueryClient()}>
      {node}
    </QueryClientProvider>,
  );

async function renderList() {
  const noop = () => {};
  const screen = await wrap(
    <>
      <button type="button">davor</button>
      <ChannelList
        server={server}
        rows={sidebarRows(server)}
        activeChannelId="wohnzimmer"
        voiceChannelId={null}
        liveChannels={{}}
        manageChannels={false}
        onEditChannel={noop}
        onCreateChannel={noop}
        onEditCategory={noop}
        onOpenProfile={noop}
        onChannelNotifications={noop}
      />
    </>,
  );
  await userEvent.click(screen.getByRole("button", { name: "davor" }));
  return (name: string) =>
    screen.getByRole("link", { name: new RegExp(`^${name}`) }).element();
}

describe("channel list keyboard", () => {
  it("is one Tab stop, on the open channel, and ↑ ↓ move between channels", async () => {
    const link = await renderList();
    await userEvent.keyboard("{Tab}");
    expect(document.activeElement).toBe(link("wohnzimmer"));

    // The heading "Sprachkanäle" is passed over; the first row keeps focus.
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(link("allgemein"));
    await userEvent.keyboard("{ArrowUp}");
    expect(document.activeElement).toBe(link("allgemein"));
    // One row per key, past "Planung"; the last row keeps focus.
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(link("wohnzimmer"));
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(link("termine"));
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(link("termine"));
  });

  it("returns Tab to the row left last", async () => {
    const link = await renderList();
    await userEvent.keyboard("{Tab}{ArrowUp}");
    expect(document.activeElement).toBe(link("allgemein"));
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(link("wohnzimmer"));
    await userEvent.keyboard("{ArrowDown}");
    expect(document.activeElement).toBe(link("termine"));
    await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
    expect(document.activeElement).not.toBe(link("termine"));
    await userEvent.keyboard("{Tab}");
    expect(document.activeElement).toBe(link("termine"));
  });

  it("leaves ↓ with a modifier to the shell", async () => {
    const link = await renderList();
    await userEvent.keyboard("{Tab}{Alt>}{ArrowDown}{/Alt}");
    expect(document.activeElement).toBe(link("wohnzimmer"));
  });
});

describe("channel list, as read out", () => {
  it("marks categories and the voice group up as headings", async () => {
    await renderList();
    for (const name of ["Sprachkanäle", "Planung"])
      await expect
        .element(page.getByRole("heading", { level: 3, name }))
        .toBeInTheDocument();
  });
});

function renderRow(
  overrides: Partial<Channel> = {},
  { live = [] as string[], manage = true } = {},
) {
  const noop = () => {};
  return wrap(
    <ChannelRow
      channel={{ ...channel("termine", "cat"), ...overrides }}
      active={false}
      inVoice={false}
      live={live}
      manage={manage}
      coarse={false}
      stop
      hintId={undefined}
      onEdit={noop}
      onMenu={noop}
    />,
  );
}

describe("channel row, as read out", () => {
  it("names a draft in the link", async () => {
    fixture.drafts = ["termine"];
    const screen = await renderRow();
    await expect
      .element(screen.getByRole("link"))
      .toHaveAccessibleName(/^termine.*Entwurf$/);
  });

  it("names who is live in a voice channel in the link", async () => {
    const screen = await renderRow(
      { kind: "voice" },
      { live: ["Mia", "Lena"] },
    );
    await expect
      .element(screen.getByRole("link"))
      .toHaveAccessibleName(/^termine.*Mia und Lena sind live$/);
  });

  it("shows a lock on a channel limited to roles, before the actions", async () => {
    for (const kind of ["text", "voice"] as const) {
      const screen = await renderRow({
        kind,
        limited: true,
        role_ids: ["r-mod"],
      });
      const lock = screen
        .getByRole("link")
        .getByRole("img", { name: "Nur für bestimmte Rollen" });
      await expect.element(lock).toBeInTheDocument();
      // The hover actions follow the link and lie over the lock.
      const actions = screen.getByRole("button", { name: /^Aktionen für / });
      expect(
        lock.element().compareDocumentPosition(actions.element()) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
      await screen.unmount();
    }
    const plain = await renderRow();
    await expect
      .element(plain.getByRole("img", { name: "Nur für bestimmte Rollen" }))
      .not.toBeInTheDocument();
  });
});

const participant = {
  kind: "participant" as const,
  key: "p-u-bob",
  member: { user_id: "u-bob", name: "Bob", avatar_url: null },
  flags: { channelId: "voice", muted: true, deafened: false },
  live: false,
};

function renderParticipant(flags: Partial<typeof participant.flags> = {}) {
  return wrap(
    <ParticipantRowView
      row={{ ...participant, flags: { ...participant.flags, ...flags } }}
      serverId="srv"
      onOpenProfile={() => {}}
    />,
  );
}

// The row is a button so a tap, the keyboard and iOS (no contextmenu on
// long-press) all open the profile with its volume controls.
describe("sidebar voice participant, as read out", () => {
  it("is a button that says what it opens and keeps the voice state", async () => {
    const screen = await renderParticipant();
    const button = screen.getByRole("button", { name: /^Bob.*stumm$/ });
    await expect
      .element(button)
      .toHaveAccessibleDescription("Profil und Lautstärke");
  });

  it("says speaking in words, not by the ring alone", async () => {
    fixture.speaking = ["u-bob"];
    const screen = await renderParticipant({ muted: false });
    await expect
      .element(screen.getByRole("button"))
      .toHaveAccessibleName(/^Bob.*spricht$/);
  });

  it("says nobody speaks who is muted or deafened", async () => {
    fixture.speaking = ["u-bob"];
    for (const flags of [{ muted: true }, { muted: false, deafened: true }]) {
      const screen = await renderParticipant(flags);
      await expect
        .element(screen.getByRole("button"))
        .not.toHaveAccessibleName(/spricht/);
      await screen.unmount();
    }
  });
});
