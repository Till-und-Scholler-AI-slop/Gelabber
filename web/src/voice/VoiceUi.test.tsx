import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { ServerDetail } from "../servers/types.ts";
import type { VoiceState } from "./session.ts";

const fixture = vi.hoisted(() => ({
  voice: {} as VoiceState,
  server: undefined as ServerDetail | undefined,
  requestedServer: undefined as string | undefined,
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
    select({ user: { id: "self" } }),
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
    select({ byServer: {}, live: fixture.live }),
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
  it("keeps global Watch stop and the original channel name outside a room", () => {
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(html).toContain('aria-label="Aktive Medien"');
    expect(html).toContain("Stage A");
    expect(html).toContain("Nicht mehr zuschauen");
    expect(html).not.toContain("Verlassen");
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
