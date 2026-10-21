import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { ServerDetail } from "../servers/types.ts";
import type { VoiceFlags } from "./roster.ts";
import type { VoiceState } from "./session.ts";

const fixture = vi.hoisted(() => ({
  voice: {} as VoiceState,
  server: undefined as ServerDetail | undefined,
  requestedServer: undefined as string | undefined,
  roster: {} as Record<string, Record<string, VoiceFlags>>,
  live: {} as Record<string, Record<string, string>>,
  speaking: { call: null as string | null, users: {} as Record<string, true> },
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
  toggleSourceWatch: vi.fn(),
  retryMicrophone: vi.fn(),
  dismissLostCall: vi.fn(),
}));
// VoiceRoom reads the user through session.ts, useUserId through store.ts,
// so both hand out the same one.
const signedIn = vi.hoisted(() => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({
      user: { id: "self", name: "Rafi", avatar_url: "/real-self.png" },
    }),
}));
vi.mock("../auth/session.ts", () => signedIn);
vi.mock("../auth/store.ts", () => signedIn);
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
vi.mock("./speaking.ts", () => ({
  useSpeakingUsers: (serverId: string, channelId: string) =>
    fixture.speaking.call === `${serverId}/${channelId}`
      ? fixture.speaking.users
      : {},
}));
vi.mock("./VoiceTile.tsx", () => ({
  VoiceTile: ({
    stream,
    label,
    speaking,
  }: {
    stream: MediaStream | null;
    label: string;
    speaking?: boolean;
  }) => (
    <span data-source={stream?.id ?? "none"} data-speaking={speaking}>
      {label}
    </span>
  ),
}));

// A static render shows a store's initial state; push-to-talk is read as it
// is set here.
vi.mock("./pushToTalk.ts", async (original) => {
  const actual = await original<typeof import("./pushToTalk.ts")>();
  const current = (select: (state: unknown) => unknown) =>
    select(actual.usePushToTalk.getState());
  return {
    ...actual,
    usePushToTalk: Object.assign(current, actual.usePushToTalk),
  };
});

import { VoiceRoom } from "./VoiceRoom.tsx";
import { ShareMenuItems, VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import { callAnnouncement } from "./callBar.ts";
import { resetPushToTalkForTests, usePushToTalk } from "./pushToTalk.ts";

const stream = { id: "source-from-a" } as MediaStream;
const server = {
  id: "a",
  role: "member",
  permissions: ["join_voice"],
  members: [],
} as unknown as ServerDetail;
/** The opening tag of the control with this name, or null. */
function control(html: string, label: string): string | null {
  return (
    html.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0] ??
    null
  );
}
/** The Teilen menu's entry with this label, through its hint, or null. */
function entry(html: string, label: string): string | null {
  return (
    html.match(
      new RegExp(
        `<button[^>]*role="menuitem"[^>]*>(?:(?!</button>).)*?<strong>${label}</strong>(?:(?!</button>).)*</button>`,
      ),
    )?.[0] ?? null
  );
}

beforeEach(() => {
  fixture.voice = {
    status: "idle",
    connection: "idle",
    mic: "live",
    lostCall: null,
    serverId: null,
    channelId: null,
    channelName: null,
    muted: false,
    deafened: false,
    camera: false,
    sharing: false,
    live: false,
    sourceWatchSupported: true,
    sourceAudio: { s: "off", l: "off" },
    sourceAudioNote: { s: null, l: null },
    sourceSubscriptions: {},
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
  // A desktop browser: the capture controls depend on what the client can do.
  vi.stubGlobal("navigator", {
    mediaDevices: { getDisplayMedia() {}, getUserMedia() {} },
  });
});
afterEach(() => {
  vi.unstubAllGlobals();
  fixture.speaking = { call: null, users: {} };
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
    expect(html).not.toContain('aria-label="Stummschalten"');
    expect(html).not.toContain("Verlassen");
    const controls = renderToStaticMarkup(<VoiceSessionControls />);
    expect(control(controls, "Stummschalten")).toContain('aria-pressed="true"');
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
    expect(html).toContain('role="status" class="sr-only"');
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
    expect(control(html, "Teilen")).toContain('aria-pressed="true"');
    expect(control(html, "Teilen")).toContain("voice-control-live");
    expect(html).toContain("Verlassen");
    const menu = (liveTakenBy: string | null = null) =>
      renderToStaticMarkup(
        <ShareMenuItems
          canGoLive={fixture.server!.permissions.includes("go_live")}
          liveTakenBy={liveTakenBy}
        />,
      );
    expect(entry(menu(), "Live beenden")).not.toBeNull();
    expect(entry(menu(), "Go Live")).toBeNull();
    fixture.voice.live = false;
    expect(entry(menu(), "Go Live")).toBeNull();
    fixture.server.permissions = ["go_live"];
    // alice is live in this channel and the members list does not name her.
    const taken = entry(menu("Jemand"), "Go Live");
    expect(taken).toContain('aria-disabled="true"');
    expect(taken).toContain("Jemand ist gerade live.");
    expect(entry(menu(), "Go Live")).not.toContain("aria-disabled");
    fixture.voice.live = true;
    expect(entry(menu("Jemand"), "Live beenden")).not.toContain(
      "aria-disabled",
    );
  });
  it("keeps one name per toggle and says its state with aria-pressed", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
    });
    const tags = () => {
      const html = renderToStaticMarkup(<VoiceControls canGoLive />);
      return ["Stummschalten", "Taubschalten", "Kamera", "Teilen"].map(
        (label) => control(html, label),
      );
    };
    for (const tag of tags()) expect(tag).toContain('aria-pressed="false"');
    Object.assign(fixture.voice, {
      muted: true,
      deafened: true,
      camera: true,
      sharing: true,
    });
    for (const tag of tags()) expect(tag).toContain('aria-pressed="true"');
    const [mic, deafen, camera, share] = tags();
    // What a press does stays in the tooltip, with its shortcut.
    expect(mic).toContain("Stummschaltung aufheben (Strg+Umschalt+M)");
    expect(deafen).toContain("Taubschaltung aufheben (Strg+Umschalt+D)");
    expect(camera).toContain('title="Kamera ausschalten"');
    // Media that is on wears the accent pill.
    expect(camera).toContain("is-on");
    expect(share).toContain("is-on");
    expect(share).toContain('aria-haspopup="menu"');
  });
  it("lists the three ways to share in the Teilen menu", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
    });
    const html = renderToStaticMarkup(<ShareMenuItems canGoLive />);
    expect(html.match(/role="menuitem"/g)).toHaveLength(2);
    expect(entry(html, "Bildschirm im Raum teilen")).toContain(
      "Sehen alle im Sprachkanal",
    );
    expect(entry(html, "Go Live")).toContain(
      "Auch für Zuschauer ohne Beitritt",
    );
    expect(html).toMatch(
      /role="menuitemcheckbox" aria-label="Ton mitsenden" aria-checked="false"/,
    );
    // While sharing, ending it comes first.
    fixture.voice.sharing = true;
    const sharing = renderToStaticMarkup(<ShareMenuItems canGoLive />);
    expect(sharing.indexOf("<strong>Teilen beenden</strong>")).toBe(
      sharing.indexOf("<strong>"),
    );
    expect(entry(sharing, "Bildschirm im Raum teilen")).toBeNull();
  });
  it("tells a missing or held-back microphone from a working one", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
      mic: "denied",
    });
    const render = () => renderToStaticMarkup(<VoiceControls />);
    expect(control(render(), "Stummschalten")).toBeNull();
    expect(control(render(), "Kein Mikrofon")).toContain(
      'aria-haspopup="dialog"',
    );
    expect(render()).toContain("voice-control-dot");
    fixture.voice.mic = "unavailable";
    expect(control(render(), "Kein Mikrofon")).not.toBeNull();
    try {
      usePushToTalk.setState({ active: true, open: false });
      fixture.voice.mic = "ptt-idle";
      const idle = render();
      expect(control(idle, "Stummschalten")).toContain("is-ptt-idle");
      expect(idle).toContain(">PTT</span>");
      usePushToTalk.setState({ open: true });
      fixture.voice.mic = "live";
      expect(control(render(), "Stummschalten")).toContain("is-talking");
    } finally {
      resetPushToTalkForTests();
    }
  });
  it("says how the connection is doing", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      channelName: "Zocken",
      watching: false,
      connection: "connecting",
    });
    fixture.server = { ...server, name: "Spieleabend" } as ServerDetail;
    const render = () => renderToStaticMarkup(<VoiceSessionControls />);
    expect(render()).toContain("Verbinde …");
    expect(render()).toContain("Zocken · Spieleabend");
    expect(render()).not.toContain("voice-quality-bars");
    fixture.voice.connection = "reconnecting";
    expect(render()).toContain("Verbindung wird wiederhergestellt …");
    fixture.voice.connection = "connected";
    expect(render()).toContain(">Verbunden<");
    expect(render()).toContain(
      'aria-label="Verbindung: wird gemessen …. Details anzeigen"',
    );
    fixture.voice.connection = "failed";
    expect(render()).toContain("Verbindung verloren");
    expect(render()).toContain("Erneut versuchen");
  });
  it("offers a lost call again after the session left it", () => {
    Object.assign(fixture.voice, {
      status: "idle",
      watching: false,
      connection: "failed",
      lostCall: { serverId: "a", channelId: "stage", channelName: "Zocken" },
    });
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(html).toContain("Verbindung verloren");
    expect(html).toContain("Zocken");
    expect(html).toContain("Erneut versuchen");
    expect(control(html, "Schließen")).not.toBeNull();
    expect(html).not.toContain("Verlassen");
    expect(html).not.toContain("Wiedergabe-Lautstärke");
  });
  it("announces joining, trouble and leaving once", () => {
    const at = (
      status: "idle" | "joined",
      connection: VoiceState["connection"],
      channelName: string | null = "Zocken",
    ) => ({ status, connection, channelName });
    expect(
      callAnnouncement(at("idle", "idle"), at("joined", "connecting")),
    ).toBeNull();
    expect(
      callAnnouncement(at("joined", "connecting"), at("joined", "connected")),
    ).toBe("Mit Zocken verbunden");
    expect(
      callAnnouncement(at("joined", "connected"), at("joined", "reconnecting")),
    ).toBe("Verbindung wird wiederhergestellt");
    expect(
      callAnnouncement(at("joined", "reconnecting"), at("joined", "connected")),
    ).toBe("Verbindung wiederhergestellt");
    expect(
      callAnnouncement(
        at("joined", "connected"),
        at("joined", "connected", "Quatschecke"),
      ),
    ).toBe("Mit Quatschecke verbunden");
    expect(
      callAnnouncement(at("joined", "connected"), at("idle", "idle")),
    ).toBe("Getrennt");
    expect(
      callAnnouncement(at("joined", "reconnecting"), at("idle", "failed")),
    ).toBe("Verbindung verloren");
    expect(
      callAnnouncement(at("idle", "failed"), at("idle", "idle")),
    ).toBeNull();
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

  it("rings whoever speaks in the joined room, never a muted member", () => {
    fixture.live = {};
    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: false, deafened: false },
        bob: { channelId: "stage", muted: true, deafened: false },
        carol: { channelId: "stage", muted: false, deafened: false },
      },
    };
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      watching: false,
      muted: true,
      participants: { alice: { pubs: ["v"] } },
      remote: { alice: { v: stream } },
    });
    fixture.speaking = {
      call: "a/stage",
      users: { alice: true, bob: true, self: true },
    };
    const members = ["alice", "bob", "carol"].map((id) => ({
      user_id: id,
      name: id[0]!.toUpperCase() + id.slice(1),
      avatar_url: null,
      joined_at: "",
      role: "member",
    }));
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{ ...server, members } as unknown as ServerDetail}
        channelId="stage"
        channelName="Stage"
      />,
    );
    const tiles = html.match(/<li class="voice-room-participant[^"]*"/g);
    // alice, bob (muted), carol, and the own seat (muted here).
    expect(tiles).toEqual([
      '<li class="voice-room-participant is-speaking"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
    ]);
    expect(html).toContain("Spricht");
    expect(html.match(/gel-avatar-lg is-speaking/g)).toHaveLength(1);
    expect(html).toContain('data-source="source-from-a" data-speaking="true"');

    // Another room of the same server shows no levels of this call.
    const other = renderToStaticMarkup(
      <VoiceRoom
        server={{ ...server, members } as unknown as ServerDetail}
        channelId="other"
        channelName="Other"
      />,
    );
    expect(other).not.toContain("is-speaking");

    fixture.server = { ...server, members } as unknown as ServerDetail;
    const dock = renderToStaticMarkup(<VoiceSessionControls />);
    expect(dock).toContain('class="voice-session-people"');
    expect(dock.match(/is-speaking/g)).toHaveLength(1);
  });
});
