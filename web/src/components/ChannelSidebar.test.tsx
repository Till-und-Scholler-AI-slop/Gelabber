import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Badge } from "../messages/notifyLevel.ts";
import { sidebarRows } from "../servers/rows.ts";
import type { Category, Channel, ServerDetail } from "../servers/types.ts";
import { resetUserVolumesForTests } from "../voice/userVolume.ts";

const fixture = vi.hoisted(() => ({
  badges: {} as Record<string, Badge>,
  drafts: [] as string[],
}));

vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  // A plain anchor with the row's attributes, without the route.
  Link: (props: Record<string, unknown>) => (
    <a
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
});
afterEach(() => resetUserVolumesForTests());

const created_at = "2026-09-13T00:00:00Z";
const category = (id: string, name: string): Category => ({
  id,
  server_id: "s1",
  name,
  created_at,
});
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
});

const server: ServerDetail = {
  id: "s1",
  name: "Spieleabend",
  owner_id: "u1",
  created_at,
  role: "owner",
  permissions: ["manage_channels"],
  member_permissions: [],
  categories: [category("cat", "Planung")],
  channels: [
    channel("allgemein", null),
    channel("wohnzimmer", null, "voice"),
    channel("termine", "cat"),
  ],
  members: [],
};

function wrap(node: ReactNode) {
  return renderToStaticMarkup(
    <QueryClientProvider client={new QueryClient()}>
      {node}
    </QueryClientProvider>,
  );
}

function list() {
  const noop = () => {};
  return wrap(
    <ChannelList
      server={server}
      rows={sidebarRows(server)}
      activeChannelId="allgemein"
      voiceChannelId={null}
      liveChannels={{}}
      manageChannels={false}
      onEditChannel={noop}
      onCreateChannel={noop}
      onEditCategory={noop}
      onOpenProfile={noop}
      onChannelNotifications={noop}
    />,
  );
}

function row({
  coarse,
  manage = true,
  voice = false,
  live = [],
  inVoice = false,
}: {
  coarse: boolean;
  manage?: boolean;
  voice?: boolean;
  live?: string[];
  inVoice?: boolean;
}) {
  const noop = () => {};
  return wrap(
    <ChannelRow
      channel={channel("termine", "cat", voice ? "voice" : "text")}
      active={false}
      inVoice={inVoice}
      live={live}
      manage={manage}
      coarse={coarse}
      stop
      hintId={undefined}
      onEdit={noop}
      onDelete={noop}
      onMenu={noop}
    />,
  );
}

describe("channel list", () => {
  it("is a list whose items know their place in it", () => {
    const html = list();
    expect(html).toContain('role="list" aria-label="Kanäle"');
    expect(html.match(/role="listitem" aria-setsize="5"/g)).toHaveLength(5);
    expect(html).toContain('aria-posinset="1"');
    expect(html).toContain('aria-posinset="5"');
  });

  it("marks categories and the voice group up as headings", () => {
    const html = list();
    expect(html).toContain(
      '<h3 class="sidebar-section-heading">Sprachkanäle</h3>',
    );
    expect(html).toContain('<h3 class="sidebar-category-name">Planung</h3>');
  });

  it("leaves the long-press hint to touch screens", () => {
    expect(list()).not.toContain("Gedrückt halten");
  });

  it("gives the list one Tab stop, on the open channel", () => {
    const links = list().match(/<a [^>]*>/g) ?? [];
    expect(links).toHaveLength(3);
    const [open, ...others] = links;
    expect(open).toContain('aria-current="page"');
    expect(open).not.toContain('tabindex="-1"');
    for (const link of others) expect(link).toContain('tabindex="-1"');
  });
});

describe("channel row", () => {
  it("marks unread channels and shows a draft", () => {
    fixture.badges = { termine: { count: 0, dot: true } };
    fixture.drafts = ["termine"];
    const html = row({ coarse: false });
    expect(html).toContain("channel-row is-unread");
    expect(html).toContain('<span class="sr-only">Entwurf</span>');
  });

  it("says who is live in a voice channel", () => {
    const html = row({ coarse: false, voice: true, live: ["Mia", "Lena"] });
    expect(html).toContain('title="Mia und Lena sind live"');
    expect(html).toContain(
      '<span class="sr-only">Mia und Lena sind live</span>',
    );
    expect(row({ coarse: false, voice: true, live: ["Mia"] })).toContain(
      "Mia ist live",
    );
    expect(row({ coarse: false, voice: true })).not.toContain("live");
  });

  it("marks the own call's channel, Live or not", () => {
    const quiet = row({ coarse: false, voice: true, inVoice: true });
    expect(quiet).toContain("channel-row is-in-voice");
    expect(quiet).toContain('title="termine · verbunden"');
    // A Live there keeps both: the badge and the green speaker.
    const live = row({
      coarse: false,
      voice: true,
      inVoice: true,
      live: ["Mia"],
    });
    expect(live).toContain("is-in-voice");
    expect(live).toContain("Mia ist live");
    expect(row({ coarse: false, voice: true })).not.toContain("is-in-voice");
  });

  it("keeps read channels plain", () => {
    const html = row({ coarse: false });
    expect(html).not.toContain("is-unread");
    expect(html).not.toContain("Entwurf");
  });

  it("shows edit and delete beside the row for a mouse", () => {
    const html = row({ coarse: false });
    expect(html).toContain('aria-label="Kanal bearbeiten"');
    expect(html).toContain('aria-label="Kanal löschen"');
    expect(html).not.toContain("Aktionen für");
  });

  it("opens the actions by a long press on a touch screen", () => {
    const html = row({ coarse: true });
    expect(html).not.toContain('aria-label="Kanal bearbeiten"');
    expect(html).not.toContain('aria-label="Kanal löschen"');
    // One button stays for screen readers, shown once a keyboard reaches it.
    expect(html).toMatch(
      /<span class="sr-only focus-within:not-sr-only"><button [^>]*aria-label="Aktionen für #termine"/,
    );
  });

  it("has no actions for members who cannot manage channels", () => {
    const html = row({ coarse: true, manage: false });
    expect(html).not.toContain("Aktionen für");
    expect(html).not.toContain("Kanal löschen");
  });
});

const participant = {
  kind: "participant" as const,
  key: "p-u-bob",
  member: { user_id: "u-bob", name: "Bob", avatar_url: null },
  flags: { channelId: "voice", muted: true, deafened: false },
  live: false,
};

// Markup only: node has no DOM to click. The row is a button so a tap,
// the keyboard and iOS (no contextmenu on long-press) all open the profile
// with its volume controls.
describe("sidebar voice participant", () => {
  it("is a button that names what it opens and keeps the voice state", () => {
    const html = renderToStaticMarkup(
      <ParticipantRowView
        row={participant}
        serverId="srv"
        onOpenProfile={() => {}}
      />,
    );
    expect(html).toMatch(
      /^<button type="button" class="sidebar-participant" title="Profil und Lautstärke"/,
    );
    expect(html).toContain("Bob");
    expect(html).toContain('<span class="sr-only">stumm</span>');
    expect(html).not.toContain("Lokal stummgeschaltet");
  });
});
