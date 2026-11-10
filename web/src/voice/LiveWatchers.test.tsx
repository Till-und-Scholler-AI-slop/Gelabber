import {
  createElement,
  createRef,
  isValidElement,
  type ReactNode,
} from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Member } from "../servers/types.ts";
import type { VoiceFlags } from "./roster.ts";

const fixture = vi.hoisted(() => ({
  voice: {} as Record<string, unknown>,
  byServer: {} as Record<string, Record<string, VoiceFlags>>,
  watches: {} as Record<string, Record<string, readonly string[]>>,
}));
const session = vi.hoisted(() => ({
  useVoice: (select: (state: unknown) => unknown) => select(fixture.voice),
  toggleShare: vi.fn(),
}));
vi.mock("./session.ts", () => session);
vi.mock("./roster.ts", () => ({
  EMPTY_OCCUPANCY: {},
  EMPTY_WATCHES: {},
  useVoiceRoster: (select: (state: unknown) => unknown) =>
    select({ byServer: fixture.byServer, watches: fixture.watches }),
}));
vi.mock("../auth/store.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "lena", name: "Lena", avatar_url: null } }),
}));
// The list inline, while open: a static render has no portal.
vi.mock("../components/ui/Popover.tsx", () => ({
  Popover: ({
    open,
    label,
    children,
  }: {
    open: boolean;
    label: string;
    children: ReactNode;
  }) => (open ? <div data-popover={label}>{children}</div> : null),
}));

import { LiveWatchCount, LiveWatchersPopover } from "./LiveWatchers.tsx";
import { useLiveWatchers } from "./useLiveWatchers.ts";

const member = (user_id: string, name: string): Member =>
  ({
    user_id,
    name,
    avatar_url: null,
    joined_at: "",
    role: "member",
    role_ids: [],
  }) as Member;
const members = [
  member("lena", "Lena"),
  member("jonas", "Jonas"),
  member("mira", "Mira"),
  member("tom", "Tom"),
  member("ada", "Ada"),
];
const flags = (channelId: string): VoiceFlags => ({
  channelId,
  muted: false,
  deafened: false,
});

beforeEach(() => {
  fixture.voice = {
    status: "joined",
    serverId: "home",
    channelId: "sofa",
    sourceSubscriptions: {},
  };
  fixture.byServer = {
    home: {
      jonas: flags("sofa"),
      tom: flags("sofa"),
      mira: flags("sofa"),
      ada: flags("sofa"),
      // In another room: never in the lists.
      bob: flags("kino"),
    },
  };
  fixture.watches = { home: { tom: ["jonas"], mira: ["jonas"] } };
  session.toggleShare.mockClear();
});

/** What a hook gives in a render. */
function hooked<T>(hook: () => T): T {
  let out: T | undefined;
  const probe = () => {
    out = hook();
    return null;
  };
  renderToStaticMarkup(createElement(probe));
  return out as T;
}

/** The props of the first element in `node` that `wanted` picks. */
function propsOf(
  node: ReactNode,
  wanted: (props: Record<string, unknown>) => boolean,
): Record<string, unknown> {
  const search = (at: ReactNode): Record<string, unknown> | null => {
    if (Array.isArray(at)) {
      for (const child of at) {
        const found = search(child);
        if (found) return found;
      }
      return null;
    }
    if (!isValidElement(at)) return null;
    const props = at.props as Record<string, unknown>;
    return wanted(props) ? props : search(props.children as ReactNode);
  };
  const found = search(node);
  if (!found) throw new Error("no such element");
  return found;
}

const count = (owner: string) =>
  renderToStaticMarkup(
    <LiveWatchCount
      serverId="home"
      channelId="sofa"
      channelName="Sofa"
      owner={owner}
      ownerName="Jonas"
      own={false}
      members={members}
    />,
  );

describe("who watches a Live", () => {
  it("splits the channel's people by name, the owner and other rooms left out", () => {
    const list = hooked(() =>
      useLiveWatchers({
        serverId: "home",
        channelId: "sofa",
        owner: "jonas",
        members,
      }),
    );
    expect(list.watching.map((person) => person.name)).toEqual(["Mira", "Tom"]);
    // The own join counts before the roster's echo, as "(du)".
    expect(list.notWatching.map((person) => person.name)).toEqual([
      "Ada",
      "Lena (du)",
    ]);
  });

  it("counts the own watch from the local state, not the gateway's echo", () => {
    fixture.voice.sourceSubscriptions = { jonas: { s: true } };
    const list = hooked(() =>
      useLiveWatchers({
        serverId: "home",
        channelId: "sofa",
        owner: "jonas",
        members,
      }),
    );
    expect(list.watching.map((person) => person.id)).toContain("lena");
  });

  it("names the count's button by how many watch, from nobody up", () => {
    expect(count("jonas")).toContain(
      'aria-label="2 schauen zu – Liste anzeigen"',
    );
    expect(count("jonas")).toContain("<span>2</span>");
    fixture.watches = { home: { tom: ["jonas"] } };
    expect(count("jonas")).toContain(
      'aria-label="1 schaut zu – Liste anzeigen"',
    );
    fixture.watches = {};
    expect(count("jonas")).toContain(
      'aria-label="Niemand schaut zu – Liste anzeigen"',
    );
    expect(count("jonas")).toContain("<span>0</span>");
    // Closed until pressed.
    expect(count("jonas")).toContain('aria-expanded="false"');
    expect(count("jonas")).not.toContain("data-popover");
  });
});

describe("the watchers' list", () => {
  const people = {
    watching: [
      { id: "mira", name: "Mira", url: null },
      { id: "tom", name: "Tom", url: null },
    ],
    notWatching: [{ id: "ada", name: "Ada", url: null }],
  };
  const props = (own: boolean, list = people) => ({
    open: true,
    onClose: vi.fn(),
    anchor: createRef<HTMLElement>(),
    placement: "top-start" as const,
    own,
    ownerName: "Jonas",
    channelName: "Sofa",
    list,
  });

  it("titles someone else's Live and lists the others apart", () => {
    const html = renderToStaticMarkup(
      <LiveWatchersPopover {...props(false)} />,
    );
    expect(html).toContain('data-popover="2 schauen zu"');
    expect(html).toContain("<strong>2 schauen zu</strong>");
    expect(html).toContain("<small>Live von Jonas in Sofa</small>");
    expect(html).toMatch(
      /Mira<\/span>.*Tom<\/span>.*Im Kanal, schaut nicht zu.*Ada/,
    );
    expect(html).not.toContain("Live beenden");
  });

  it("hides the others' section when everyone watches", () => {
    const html = renderToStaticMarkup(
      <LiveWatchersPopover
        {...props(false, { watching: people.watching, notWatching: [] })}
      />,
    );
    expect(html).not.toContain("Im Kanal, schaut nicht zu");
    const nobody = renderToStaticMarkup(
      <LiveWatchersPopover
        {...props(false, { watching: [], notWatching: people.notWatching })}
      />,
    );
    expect(nobody).toContain("<strong>Niemand schaut zu</strong>");
  });

  it("ends the own Live from its list", () => {
    const own = props(true);
    const html = renderToStaticMarkup(
      <LiveWatchersPopover {...own} extra={<i data-extra="" />} />,
    );
    expect(html).toContain("<small>Dein Live in Sofa</small>");
    expect(html).toMatch(/role="separator".*Live beenden.*data-extra/);
    let tree: ReactNode = null;
    hooked(() => {
      tree = LiveWatchersPopover(own);
      return null;
    });
    const end = propsOf(tree, (p) =>
      String(p.className ?? "").includes("voice-watchers-end"),
    );
    (end.onClick as () => void)();
    expect(own.onClose).toHaveBeenCalled();
    expect(session.toggleShare).toHaveBeenCalledTimes(1);
  });
});
