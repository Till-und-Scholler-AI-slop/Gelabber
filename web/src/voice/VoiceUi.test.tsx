import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ServerDetail } from "../servers/types.ts";
import type { VoiceFlags } from "./roster.ts";
import type { VoiceState } from "./session.ts";

const fixture = vi.hoisted(() => ({
  voice: {} as VoiceState,
  server: undefined as ServerDetail | undefined,
  requestedServer: undefined as string | undefined,
  roster: {} as Record<string, Record<string, VoiceFlags>>,
  live: {} as Record<string, Record<string, string>>,
}));
vi.mock("./session.ts", () => ({
  useVoice: (select?: (state: VoiceState) => unknown) =>
    select ? select(fixture.voice) : fixture.voice,
  joinVoice: vi.fn(),
  stopWatching: vi.fn(),
  watchLive: vi.fn(),
  retryPlayback: vi.fn(),
  leaveVoice: vi.fn(),
  toggleCamera: vi.fn(),
  toggleDeafen: vi.fn(),
  toggleGoLive: vi.fn(),
  toggleMute: vi.fn(),
  toggleShare: vi.fn(),
}));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({
      user: { id: "self", name: "Rafi", avatar_url: "/real-self.png" },
    }),
}));
vi.mock("../servers/queries.ts", () => ({
  useServer: (id: string | undefined) => {
    fixture.requestedServer = id;
    return { data: fixture.server };
  },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));
vi.mock("./roster.ts", () => ({
  EMPTY_OCCUPANCY: {},
  useVoiceRoster: (select: (state: unknown) => unknown) =>
    select({ byServer: fixture.roster, live: fixture.live }),
  liveOf: (live: typeof fixture.live, server: string, channel: string) =>
    live[server]?.[channel] ?? null,
}));
vi.mock("./VoiceTile.tsx", () => ({
  VoiceTile: ({
    stream,
    label,
  }: {
    stream: MediaStream | null;
    label: string;
  }) => <span data-source={stream?.id ?? "none"}>{label}</span>,
}));

import { VoiceRoom } from "./VoiceRoom.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";

const stream = { id: "source-from-a" } as MediaStream;
const server = {
  id: "a",
  role: "member",
  permissions: ["join_voice"],
  members: [],
} as unknown as ServerDetail;
beforeEach(() => {
  fixture.voice = {
    status: "idle",
    serverId: null,
    channelId: null,
    channelName: null,
    muted: false,
    deafened: false,
    camera: false,
    sharing: false,
    live: false,
    localCamera: null,
    localScreen: null,
    localLive: null,
    remote: {},
    participants: {},
    watching: true,
    watchServerId: "a",
    watchChannelId: "stage",
    watchChannelName: "Stage A",
    watchPublisherId: "alice",
    watchStream: stream,
    playbackBlocked: false,
  };
  fixture.roster = {};
  fixture.live = { a: { stage: "alice" }, b: { stage: "bob" } };
  fixture.server = undefined;
  fixture.requestedServer = undefined;
});

describe("active media UI", () => {
  it("renders only the Watch session's exact server, channel and publisher", () => {
    const render = (id: string, channel = "stage") =>
      renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, id }}
          channelId={channel}
          channelName="Stage"
        />,
      );
    expect(render("a")).toContain('data-source="source-from-a"');
    expect(render("b")).not.toContain('data-source="source-from-a"');
    fixture.live.a!.other = "alice";
    expect(render("a", "other")).not.toContain('data-source="source-from-a"');
    fixture.live.a!.stage = "bob";
    expect(render("a")).not.toContain('data-source="source-from-a"');
  });
  it("does not render Voice remote media outside its active room", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
      remote: { bob: { l: stream } },
    });
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{ ...server, id: "b" }}
        channelId="stage"
        channelName="Stage B"
      />,
    );
    expect(html).not.toContain('data-source="source-from-a"');
    expect(html).toContain("Beitreten");
  });
  it("binds Live to the current publisher even if an old publication remains in the seat", () => {
    fixture.live = { a: { stage: "bob" } };
    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: false, deafened: false },
        bob: { channelId: "stage", muted: false, deafened: false },
      },
    };
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
      participants: { alice: { pubs: ["l"] }, bob: { pubs: ["l"] } },
      remote: { alice: { l: stream }, bob: { l: { id: "current-publisher" } } },
    });
    const html = renderToStaticMarkup(
      <VoiceRoom server={server} channelId="stage" channelName="Stage" />,
    );
    expect(html).not.toContain('data-source="source-from-a"');
    expect(html).toContain('data-source="current-publisher"');
  });

  it("shows real avatars and disconnected self, without fictional activity or capture", () => {
    fixture.live = {};
    fixture.voice.watching = false;
    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: true, deafened: false },
        bob: { channelId: "other", muted: false, deafened: false },
      },
    };
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{
          ...server,
          members: [
            {
              user_id: "alice",
              name: "Alice",
              avatar_url: "/real-alice.png",
              joined_at: "",
              role: "member",
            },
            {
              user_id: "bob",
              name: "Bob",
              avatar_url: null,
              joined_at: "",
              role: "member",
            },
          ],
        }}
        channelId="stage"
        channelName="Wohnzimmer"
      />,
    );
    expect(html).toContain('class="voice-room"');
    expect(html).toContain("Alice");
    expect(html).toContain("/real-alice.png");
    expect(html).toContain("Stumm");
    expect(html).toContain("Rafi (du)");
    expect(html).toContain("Du bist noch nicht verbunden");
    expect(html).toContain("Dazukommen");
    expect(html).toContain('aria-label="Beitreten"');
    expect(html).toContain("Mikrofon testen");
    expect(html).not.toContain("Bob");
    expect(html).not.toContain("Spricht gerade");
    expect(html).not.toContain("Zuschauen");
    expect(html).not.toContain("Mikrofonpegel");
  });

  it("shows the immediately joined self once, before the roster echo", () => {
    fixture.live = {};
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      muted: true,
      deafened: true,
      watching: false,
    });
    const html = renderToStaticMarkup(
      <VoiceRoom server={server} channelId="stage" channelName="Wohnzimmer" />,
    );
    expect(html.match(/Rafi \(du\)/g)).toHaveLength(1);
    expect(html).toContain("Taub");
    expect(html).not.toContain("Du bist noch nicht verbunden");
    expect(html).not.toContain("Dazukommen");
    expect(html).not.toContain('aria-label="Mikrofon aus"');
    expect(html).not.toContain("Verlassen");
    const controls = renderToStaticMarkup(<VoiceSessionControls />);
    expect(controls).toContain('aria-label="Mikrofon an"');
    expect(controls).toContain("Verlassen");
  });

  it("keeps camera and screen media scoped to the exact active server and channel", () => {
    fixture.live = {};
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
      b: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      participants: { alice: { pubs: ["v", "s"] } },
      remote: { alice: { v: stream, s: stream } },
      watching: false,
    });
    const render = (id: string, channelId = "stage") =>
      renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, id }}
          channelId={channelId}
          channelName="Room"
        />,
      );
    expect(render("a").match(/data-source="source-from-a"/g)).toHaveLength(2);
    expect(render("a")).toContain("Raster");
    expect(render("a")).toContain("Bildschirm");
    expect(render("a")).toContain("Kamera");
    expect(render("b")).not.toContain('data-source="source-from-a"');
    expect(render("a", "other")).not.toContain('data-source="source-from-a"');
  });

  it("respects join rights and keeps the Watch stop action in the persistent bar", () => {
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{ ...server, permissions: [] }}
        channelId="stage"
        channelName="Stage"
      />,
    );
    expect(html).not.toContain('aria-label="Beitreten"');
    expect(html).toContain("kein Recht");
    expect(html).not.toContain("Nicht mehr zuschauen");
    expect(renderToStaticMarkup(<VoiceSessionControls />)).toContain(
      "Nicht mehr zuschauen",
    );
    expect(html).toContain("Mikrofon testen");
  });

  it("keeps global Watch stop and the original channel name outside a room", () => {
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(html).toContain('aria-label="Aktive Medien"');
    expect(html).toContain("Stage A");
    expect(html).toContain("Nicht mehr zuschauen");
    expect(html).not.toContain("Verlassen");
    expect(html).toContain('aria-label="Wiedergabe-Lautstärke"');
    expect(html).toContain('aria-label="Voice-Einstellungen"');
  });
  it("uses active Voice server rights and always exposes Live stop and Leave", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      channelName: "Voice A",
      live: true,
    });
    fixture.server = { ...server, permissions: [] };
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(fixture.requestedServer).toBe("a");
    expect(html).toContain("Voice A");
    expect(html).toContain('aria-label="Live beenden"');
    expect(html).toContain("Verlassen");
    expect(html).not.toContain('aria-label="Go Live"');
    fixture.voice.live = false;
    expect(renderToStaticMarkup(<VoiceSessionControls />)).not.toContain(
      'aria-label="Go Live"',
    );
    fixture.server.permissions = ["go_live"];
    expect(renderToStaticMarkup(<VoiceSessionControls />)).toContain(
      'aria-label="Go Live"',
    );
  });
  it("shows a playback action while blocked and hides it while deafened", () => {
    fixture.voice.playbackBlocked = true;
    expect(renderToStaticMarkup(<VoiceSessionControls />)).toContain(
      "Ton starten",
    );
    fixture.voice.deafened = true;
    expect(renderToStaticMarkup(<VoiceSessionControls />)).not.toContain(
      "Ton starten",
    );
  });
});
