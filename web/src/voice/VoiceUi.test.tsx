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
  live: {} as Record<
    string,
    Record<string, Record<string, Partial<Record<"s" | "l", true>>>>
  >,
  speaking: { call: null as string | null, users: {} as Record<string, true> },
}));
vi.mock("./session.ts", () => ({
  useVoice: (select?: (state: VoiceState) => unknown) =>
    select ? select(fixture.voice) : fixture.voice,
  joinVoice: vi.fn(),
  retryPlayback: vi.fn(),
  leaveVoice: vi.fn(),
  toggleCamera: vi.fn(),
  toggleDeafen: vi.fn(),
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
  EMPTY_LIVE_PEOPLE: {},
  useVoiceRoster: (select: (state: unknown) => unknown) =>
    select({ byServer: fixture.roster, live: fixture.live }),
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

vi.mock("./userVolume.ts", async (original) => {
  const actual = await original<typeof import("./userVolume.ts")>();
  const current = (select: (state: unknown) => unknown) =>
    select(actual.useUserVolumes.getState());
  return {
    ...actual,
    useUserVolumes: Object.assign(current, actual.useUserVolumes),
  };
});

import { VoiceRoom } from "./VoiceRoom.tsx";
import { LiveMenuItems, VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import { callAnnouncement } from "./callBar.ts";
import { CallSound } from "./QuickCallSettings.tsx";
import { resetPushToTalkForTests, usePushToTalk } from "./pushToTalk.ts";
import { resetUserVolumesForTests, useUserVolumes } from "./userVolume.ts";

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
/** The Live menu's entry with this label, through its hint, or null. */
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
    sourceWatchSupported: true,
    sourceAudio: { s: "off" },
    sourceAudioNote: { s: null },
    sourceSubscriptions: {},
    localCamera: null,
    localScreen: null,
    remote: {},
    participants: {},
    playbackBlocked: false,
  };
  fixture.roster = {};
  fixture.live = {
    a: { stage: { alice: { l: true } } },
    b: { stage: { bob: { s: true } } },
  };
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
  it("says who is live outside the room, with no pictures and no watching from outside", () => {
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{
          ...server,
          members: [
            {
              user_id: "alice",
              name: "Alice",
              avatar_url: null,
              joined_at: "",
              role: "member",
            },
          ],
        }}
        channelId="stage"
        channelName="Stage"
      />,
    );
    expect(html).toContain(
      "Alice ist da. Alice ist live – komm dazu, um zuzusehen.",
    );
    expect(html).toContain("Im Raum · Live");
    // Watching takes coming along: the way in is the only action.
    expect(html).toContain("Dazukommen</button>");
    expect(html).not.toContain("Zuschauen");
    expect(html).not.toContain('aria-label="Übertragungen"');
    expect(html).not.toContain("data-source=");
  });
  it("does not render Voice remote media outside its active room", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
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
    // Joined elsewhere, the way in says what it does, and is its own name.
    expect(html).toContain("Hierher wechseln</button>");
    expect(html).not.toContain('aria-label="Beitreten"');
  });
  it("puts every Live in the room on the stage", () => {
    fixture.live = { a: { stage: { bob: { s: true }, alice: { s: true } } } };
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
      participants: { alice: { pubs: ["s"] }, bob: { pubs: ["s"] } },
      remote: { alice: { s: stream }, bob: { s: { id: "bob-screen" } } },
    });
    const html = renderToStaticMarkup(
      <VoiceRoom server={server} channelId="stage" channelName="Stage" />,
    );
    expect(html).toContain('data-source="source-from-a"');
    expect(html).toContain('data-source="bob-screen"');
    expect(html.match(/— Live</g)).toHaveLength(2);
    // Whoever went live first holds the spotlight.
    expect(html).toMatch(
      /<div class="voice-room-spotlight"><span data-source="bob-screen"/,
    );
    expect(html.match(/· Live</g)).toHaveLength(2);
  });

  it("shows real avatars and disconnected self, without fictional activity or capture", () => {
    fixture.live = {};
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
    // One line on who is there, and a free seat instead of a card for self.
    expect(html).toContain("Alice ist da.");
    expect(html).not.toContain("Rafi (du)");
    expect(html).not.toContain("noch nicht verbunden");
    expect(html.match(/<li class="voice-room-seat"/g)).toHaveLength(1);
    expect(html).toContain("Platz frei");
    // The visible text is the button's name.
    expect(html).toMatch(
      /<button[^>]*>(?:<svg.*?<\/svg>)?Dazukommen<\/button>/,
    );
    expect(html).not.toContain('aria-label="Beitreten"');
    expect(html).toContain('aria-label="Mikrofon testen"');
    expect(html).toContain('aria-label="Voice-Einstellungen"');
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
    });
    const render = (id: string, channelId = "stage") =>
      renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, id }}
          channelId={channelId}
          channelName="Room"
        />,
      );
    const html = render("a");
    expect(html.match(/data-source="source-from-a"/g)).toHaveLength(2);
    // The share on the stage, above the people; the camera in its owner's
    // place, so Alice is one list entry. No chips to switch layouts.
    expect(html.indexOf('aria-label="Übertragungen"')).toBeLessThan(
      html.indexOf('aria-label="Teilnehmer im Sprachkanal"'),
    );
    expect(html).toContain('<div class="voice-room-spotlight">');
    expect(html.match(/<li class="voice-room-participant/g)).toHaveLength(2);
    expect(html).toMatch(
      /<li class="voice-room-participant has-video"(?:(?!<\/li>).)*data-source="source-from-a"/,
    );
    expect(html).toContain("Im Raum · Live");
    expect(html).not.toContain("Bildschirm");
    expect(html).not.toContain("Raster");
    expect(render("b")).not.toContain('data-source="source-from-a"');
    expect(render("a", "other")).not.toContain('data-source="source-from-a"');
  });

  it("respects join rights, and offers no watching from outside", () => {
    const html = renderToStaticMarkup(
      <VoiceRoom
        server={{ ...server, permissions: [] }}
        channelId="stage"
        channelName="Stage"
      />,
    );
    expect(html).not.toContain('aria-label="Beitreten"');
    expect(html).toContain("kein Recht");
    expect(html).not.toMatch(/zuschauen|Zuschauen/);
    expect(html).toContain("Mikrofon testen");
  });

  it("shows no bar outside a call", () => {
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(html).toContain('role="status" class="sr-only"');
    expect(html).not.toContain('aria-label="Aktive Medien"');
    expect(html).not.toMatch(/zuschauen|Zuschauen/);
  });
  it("uses active Voice server rights and always exposes Live stop and Leave", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      channelName: "Voice A",
      sharing: true,
    });
    fixture.server = { ...server, permissions: [] };
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(fixture.requestedServer).toBe("a");
    expect(html).toContain("Voice A");
    expect(control(html, "Live gehen")).toContain('aria-pressed="true"');
    expect(control(html, "Live gehen")).toContain("voice-control-live");
    expect(control(html, "Live gehen")).not.toContain("opacity-50");
    expect(html).toContain("Verlassen");
    const menu = () =>
      renderToStaticMarkup(
        <LiveMenuItems
          canLive={fixture.server!.permissions.includes("go_live")}
        />,
      );
    expect(entry(menu(), "Live beenden")).not.toContain("aria-disabled");
    expect(entry(menu(), "Live gehen")).toBeNull();
    fixture.voice.sharing = false;
    // Without go_live the way in stays in sight and says why it is shut.
    const denied = entry(menu(), "Live gehen");
    expect(denied).toContain('aria-disabled="true"');
    expect(denied).toContain("In diesem Server darfst du nicht live gehen.");
    expect(
      control(renderToStaticMarkup(<VoiceSessionControls />), "Live gehen"),
    ).toContain("opacity-50");
    fixture.server.permissions = ["go_live"];
    expect(entry(menu(), "Live gehen")).not.toContain("aria-disabled");
    expect(entry(menu(), "Live gehen")).toContain("Sehen alle im Sprachkanal");
  });
  it("keeps the volumes behind one speaker while a Live plays", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "b",
      channelId: "stage",
      channelName: "Voice B",
    });
    fixture.server = server;
    const quiet = renderToStaticMarkup(<VoiceSessionControls />);
    expect(control(quiet, "Lautstärke")).toBeNull();
    fixture.voice.sourceSubscriptions = { bob: { s: true } };
    const html = renderToStaticMarkup(<VoiceSessionControls />);
    expect(control(html, "Lautstärke")).toContain('aria-haspopup="dialog"');
    // No slider across the bar: the volumes open from the speaker.
    expect(html).not.toContain('type="range"');
    expect(html).not.toContain("voice-session-mix");
  });
  it("rests both volumes while deafened and says why", () => {
    fixture.voice.deafened = true;
    const html = renderToStaticMarkup(<CallSound />);
    expect(html.match(/<input[^>]*disabled=""/g)).toHaveLength(2);
    expect(html).toContain("Taubgeschaltet.");
    // The stream's mute stays in reach for after.
    expect(control(html, "Stream-Ton stummschalten")).not.toContain("disabled");
  });
  it("keeps one name per toggle and says its state with aria-pressed", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    const tags = () => {
      const html = renderToStaticMarkup(<VoiceControls canLive />);
      return ["Stummschalten", "Taubschalten", "Kamera", "Live gehen"].map(
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
    const [mic, deafen, camera, live] = tags();
    // What a press does stays in the tooltip, with its shortcut.
    expect(mic).toContain("Stummschaltung aufheben (Strg+Umschalt+M)");
    expect(deafen).toContain("Taubschaltung aufheben (Strg+Umschalt+D)");
    expect(mic).toContain('aria-keyshortcuts="Control+Shift+M"');
    expect(deafen).toContain('aria-keyshortcuts="Control+Shift+D"');
    expect(camera).toContain('title="Kamera ausschalten"');
    // A camera that is on wears the accent pill, a Live the red one.
    expect(camera).toContain("is-on");
    expect(live).toContain("voice-control-live");
    expect(live).toContain('title="Du bist live"');
    expect(live).toContain('aria-haspopup="menu"');
  });
  it("offers going live and its sound in the Live menu", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    const html = renderToStaticMarkup(<LiveMenuItems canLive />);
    expect(html.match(/role="menuitem"/g)).toHaveLength(1);
    expect(entry(html, "Live gehen")).toContain("Sehen alle im Sprachkanal");
    expect(html).toMatch(
      /role="menuitemcheckbox" aria-label="Ton mitsenden" aria-checked="false"/,
    );
    expect(html).not.toMatch(/Go Live|Bildschirm teilen|Teilen/);
    // While live, ending it takes the first place.
    fixture.voice.sharing = true;
    const live = renderToStaticMarkup(<LiveMenuItems canLive />);
    expect(live.indexOf("<strong>Live beenden</strong>")).toBe(
      live.indexOf("<strong>"),
    );
    expect(entry(live, "Live gehen")).toBeNull();
  });
  it("tells a missing or held-back microphone from a working one", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
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
      'aria-label="Verbindung: wird gemessen … – Details anzeigen"',
    );
    // The bars open the connection at a glance, not the dialog.
    expect(render()).toMatch(
      /class="voice-quality-bars"[^>]*aria-haspopup="dialog" aria-expanded="false"/,
    );
    fixture.voice.connection = "failed";
    expect(render()).toContain("Verbindung verloren");
    expect(render()).toContain("Erneut versuchen");
  });
  it("offers a lost call again after the session left it", () => {
    Object.assign(fixture.voice, {
      status: "idle",
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
      '<li class="voice-room-participant is-speaking has-video"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
    ]);
    expect(html).toContain("Spricht");
    // Alice's camera is in her entry and carries the one ring.
    expect(html).not.toContain("gel-avatar-lg is-speaking");
    expect(html).toMatch(
      /<li class="voice-room-participant is-speaking has-video"(?:(?!<\/li>).)*data-source="source-from-a" data-speaking="true"/,
    );

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

  it("seats the first one in, and keeps everyone's volume one press away", () => {
    fixture.live = {};
    const members = ["alice", "bob"].map((id) => ({
      user_id: id,
      name: id[0]!.toUpperCase() + id.slice(1),
      avatar_url: null,
      joined_at: "",
      role: "member",
    }));
    const room = () =>
      renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, members } as unknown as ServerDetail}
          channelId="stage"
          channelName="Stage"
          named
        />,
      );
    const empty = room();
    expect(empty).toContain('<h2 class="voice-room-name">Stage</h2>');
    expect(empty).toContain("Noch niemand da – komm als Erste:r dazu.");
    expect(empty.match(/<li class="voice-room-seat"/g)).toHaveLength(3);

    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: false, deafened: false },
        bob: { channelId: "stage", muted: false, deafened: false },
      },
    };
    useUserVolumes.getState().setVolume("bob", 0.7);
    try {
      const html = room();
      expect(html).toContain("Alice und Bob sind da.");
      expect(html).toContain('aria-label="Optionen für Alice"');
      expect(html).toContain('aria-label="Optionen für Bob"');
      // Bob is quieter here: the card says so.
      expect(html).toMatch(
        /class="voice-room-volume-chip"[^>]*><span class="sr-only">Lautstärke <\/span>70 %/,
      );
      expect(html.match(/voice-room-volume-chip/g)).toHaveLength(1);
    } finally {
      resetUserVolumesForTests();
    }
  });
});
