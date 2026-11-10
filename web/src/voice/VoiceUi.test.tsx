import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useLocale } from "../i18n/locale.ts";
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
  /** The roster's watches: per server, per watcher, whose Lives. */
  watches: {} as Record<string, Record<string, readonly string[]>>,
  speaking: { call: null as string | null, users: {} as Record<string, true> },
  /** The window's width for `max-width` queries; null matches none. */
  width: null as number | null,
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
// The invite dialog as a mark: whether VoiceRoom keeps it mounted.
vi.mock("../components/InviteDialog.tsx", () => ({
  InviteDialog: ({ open }: { open: boolean }) => (
    <i data-invite-dialog={open ? "open" : "closed"} />
  ),
}));
vi.mock("../components/useMediaQuery.ts", () => ({
  useMediaQuery: (query: string) => {
    const max = /max-width: (\d+)px/.exec(query)?.[1];
    return fixture.width !== null && max !== undefined && fixture.width <= +max;
  },
}));
vi.mock("./roster.ts", () => ({
  EMPTY_OCCUPANCY: {},
  EMPTY_LIVE_PEOPLE: {},
  EMPTY_WATCHES: {},
  useVoiceRoster: (select: (state: unknown) => unknown) =>
    select({
      byServer: fixture.roster,
      live: fixture.live,
      watches: fixture.watches,
    }),
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
import {
  LiveMenuItems,
  VoiceAudioToggles,
  VoiceControls,
} from "../components/VoiceControls.tsx";
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
    sourceSubscriptions: {},
    localCamera: null,
    localScreen: null,
    remote: {},
    participants: {},
    playbackBlocked: false,
  };
  fixture.roster = {};
  fixture.watches = {};
  fixture.live = {
    a: { stage: { alice: { l: true } } },
    b: { stage: { bob: { s: true } } },
  };
  fixture.server = undefined;
  fixture.requestedServer = undefined;
  fixture.width = null;
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
              role_ids: [],
            },
          ],
        }}
        channelId="stage"
        channelName="Stage"
      />,
    );
    expect(html).toContain("Alice ist da. Alice ist live.");
    expect(html).toContain("Im Raum · Live");
    // Watching takes coming along: the way in is the only action.
    expect(html).toContain("Beitreten</button>");
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
    // Someone is live: Fokus, whoever went live first in front, the other
    // Live and the people in the row under it.
    expect(html).toContain('data-view="fokus"');
    expect(html).toMatch(
      /<li class="voice-stage-item is-focus">(?:(?!<\/li>).)*data-source="bob-screen"/,
    );
    expect(html).toMatch(
      /<li class="voice-stage-item is-strip">(?:(?!<\/li>).)*data-source="source-from-a"/,
    );
    expect(
      html.match(
        /<li class="voice-room-participant voice-stage-item is-strip"/g,
      ),
    ).toHaveLength(3);
    expect(html).not.toContain('aria-label="Ohne Kamera"');
    expect(html).toContain(
      '<p class="voice-header-facts">3 im Kanal · 2 Live</p>',
    );
    // The row says who is live, in a line of its own (#521).
    expect(html.match(/>Ist live<\/span>/g)).toHaveLength(2);
    // The overview keeps the spotlight for whoever went live first.
    const overview = renderToStaticMarkup(
      <VoiceRoom
        server={server}
        channelId="stage"
        channelName="Stage"
        overview
      />,
    );
    expect(overview).toMatch(
      /<div class="voice-room-spotlight"><span data-source="bob-screen"/,
    );
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
              role_ids: [],
            },
            {
              user_id: "bob",
              name: "Bob",
              avatar_url: null,
              joined_at: "",
              role: "member",
              role_ids: [],
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
    expect(html).not.toContain("(du)");
    expect(html).not.toContain("noch nicht verbunden");
    expect(html.match(/<li class="voice-room-seat"/g)).toHaveLength(1);
    expect(html).toContain("Platz frei");
    // The visible text is the button's name.
    expect(html).toMatch(/<button[^>]*>(?:<svg.*?<\/svg>)?Beitreten<\/button>/);
    expect(html).not.toContain('aria-label="Beitreten"');
    expect(html).toContain('aria-label="Mikrofon testen"');
    expect(html).toContain('aria-label="Anrufeinstellungen"');
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
    expect(
      html.match(/Rafi<span class="voice-room-self"> \(du\)<\/span>/g),
    ).toHaveLength(1);
    expect(html).toContain("Taub");
    expect(html).not.toContain("Du bist noch nicht verbunden");
    expect(html).not.toContain("Beitreten");
    expect(html).not.toContain('aria-label="Stummschalten"');
    expect(html).not.toContain("Verlassen");
    const controls = renderToStaticMarkup(<VoiceSessionControls />);
    expect(control(controls, "Stummschalten")).toContain('aria-pressed="true"');
    expect(controls).toContain("Verlassen");
  });

  it("names the stage views Grid, Focus and Cinema in English", () => {
    fixture.live = {};
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      participants: { alice: { pubs: ["v", "s"] } },
      remote: { alice: { v: stream, s: stream } },
    });
    useLocale.setState({ locale: "en" });
    try {
      const html = renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, id: "a" }}
          channelId="stage"
          channelName="Room"
        />,
      );
      expect(html).toMatch(
        /<button type="button" class="voice-stage-kino" aria-keyshortcuts="K" title="Cinema \(K\)"[^>]*>(?:<svg.*?<\/svg>)?Cinema<\/button>/,
      );
      expect(html).toMatch(
        /role="radiogroup" aria-label="View"(?:(?!<\/div>).)*>(?:<svg.*?<\/svg>)?Grid(?:(?!<\/div>).)*>(?:<svg.*?<\/svg>)?Focus/,
      );
      expect(html).toContain("2 in the channel · 1 live");
    } finally {
      useLocale.setState({ locale: "de" });
    }
  });

  it("draws the channel header with the facts, the view and Kino in the call", () => {
    fixture.live = { a: { stage: { alice: { s: true } } } };
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    const page = () =>
      renderToStaticMarkup(
        <VoiceRoom
          server={server}
          channelId="stage"
          channelName="Stage"
          title={<h1>Stage</h1>}
          hint={<i data-live-pill="" />}
          actions={<i data-members-toggle="" />}
        />,
      );
    const header = (html: string) =>
      /<header class="lr-channel-header">(.*?)<\/header>/.exec(html)?.[1] ?? "";

    // Outside the call: the name, the Live pill and the members, as shipped;
    // the room's line stays in the room.
    const outside = page();
    expect(header(outside)).toBe(
      '<h1>Stage</h1><i data-live-pill=""></i><span class="lr-channel-header-spacer"></span><i data-members-toggle=""></i>',
    );
    expect(outside).toContain(
      '<p class="voice-room-summary">Jemand ist da. Jemand ist live.</p>',
    );

    // In the call with a Live: facts after the name, then the view and
    // Kino before the members; no pill and no bar over the stage.
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      participants: { alice: { pubs: ["s"] } },
      remote: { alice: { s: stream } },
    });
    const staged = page();
    expect(header(staged)).toMatch(
      /^<h1>Stage<\/h1><p class="voice-header-facts">2 im Kanal · 1 Live<\/p><span class="lr-channel-header-spacer"><\/span><div role="radiogroup" aria-label="Ansicht" class="ui-chip-group voice-stage-views">.*<button type="button" class="voice-stage-kino"[^>]*>.*Kino<\/button><i data-members-toggle=""><\/i>$/,
    );
    expect(staged).not.toContain("data-live-pill");
    expect(staged).not.toContain("voice-stage-bar");
    expect(staged).not.toContain("voice-room-summary");
    // The header sits over the room, outside its scrolling body.
    expect(staged).toMatch(
      /<\/header><div class="min-h-0 flex-1 overflow-y-auto"><section class="voice-room"/,
    );

    // In the call without pictures: who is there, and nothing to switch.
    fixture.voice.participants = {};
    fixture.voice.remote = {};
    fixture.live = {};
    const people = page();
    expect(header(people)).toBe(
      '<h1>Stage</h1><p class="voice-header-facts">Du bist hier mit Jemand.</p><span class="lr-channel-header-spacer"></span><i data-members-toggle=""></i>',
    );
    expect(people).not.toContain("voice-room-summary");
  });

  it("keeps only the name and the members in a phone's header", () => {
    fixture.width = 390;
    fixture.live = { a: { stage: { alice: { s: true } } } };
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
      participants: { alice: { pubs: ["s"] } },
      remote: { alice: { s: stream } },
    });
    const page = () =>
      renderToStaticMarkup(
        <VoiceRoom
          server={server}
          channelId="stage"
          channelName="Stage"
          title={<h1>Stage</h1>}
          hint={<i data-live-pill="" />}
          actions={<i data-members-toggle="" />}
        />,
      );
    const staged = page();
    expect(staged).toContain(
      '<header class="lr-channel-header"><h1>Stage</h1><span class="lr-channel-header-spacer"></span><i data-members-toggle=""></i></header>',
    );
    // The facts open the room's body instead.
    expect(staged).toMatch(
      /<section class="voice-room"[^>]*><p class="voice-room-summary">2 im Kanal · 1 Live<\/p>/,
    );

    fixture.voice.participants = {};
    fixture.voice.remote = {};
    fixture.live = {};
    expect(page()).toMatch(
      /<section class="voice-room"[^>]*><p class="voice-room-summary">Du bist hier mit Jemand\.<\/p>/,
    );
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
    // Live and camera are tiles of the stage: Alice is live, so her Live
    // is in front and her camera, her entry, in the row with the own seat.
    expect(html).not.toContain("voice-room-spotlight");
    expect(html.match(/<li class="voice-room-participant/g)).toHaveLength(2);
    expect(html).toMatch(
      /<li class="voice-room-participant has-video voice-stage-item is-strip"(?:(?!<\/li>).)*data-source="source-from-a"/,
    );
    expect(html).toContain("Im Raum · Live");
    expect(html).not.toContain("Bildschirm");
    // Kino, by its button or K.
    expect(html).toMatch(
      /<button type="button" class="voice-stage-kino" aria-keyshortcuts="K"[^>]*>(?:<svg.*?<\/svg>)?Kino<\/button>/,
    );
    // The views to switch between, Fokus picked by the Live.
    expect(html).toMatch(
      /role="radiogroup" aria-label="Ansicht"(?:(?!<\/div>).)*aria-checked="false"[^>]*>(?:<svg.*?<\/svg>)?Raster(?:(?!<\/div>).)*aria-checked="true"[^>]*>(?:<svg.*?<\/svg>)?Fokus/,
    );
    // The overview: the share in its spotlight, the camera in its owner's
    // card.
    const overview = renderToStaticMarkup(
      <VoiceRoom
        server={server}
        channelId="stage"
        channelName="Room"
        overview
      />,
    );
    expect(overview.indexOf('aria-label="Übertragungen"')).toBeLessThan(
      overview.indexOf('aria-label="Teilnehmer im Sprachkanal"'),
    );
    expect(overview).toContain('<div class="voice-room-spotlight">');
    expect(overview).toMatch(
      /<li class="voice-room-participant has-video"(?:(?!<\/li>).)*data-source="source-from-a"/,
    );
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
    expect(html).toContain("Keine Berechtigung für Sprachkanäle.");
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
    expect(control(html, "Live-Ton stummschalten")).not.toContain("disabled");
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
    // The shortcut is the control's; what a press does is in its styled
    // tooltip (callTip), not in a browser `title`.
    expect(mic).toContain('aria-keyshortcuts="Control+Shift+M"');
    expect(deafen).toContain('aria-keyshortcuts="Control+Shift+D"');
    for (const tag of tags()) expect(tag).not.toContain("title=");
    // A camera that is on wears the accent pill, a Live the red one.
    expect(camera).toContain("is-on");
    expect(live).toContain("voice-control-live");
    // While live a press lists the watchers, with "Live beenden" (#521).
    expect(live).toContain('aria-haspopup="dialog"');
  });
  it("says Stumm or Taub beside a closed microphone or headset", () => {
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    const words = (html = renderToStaticMarkup(<VoiceControls />)) => {
      return ["Stummschalten", "Taubschalten"].map(
        (label) =>
          html
            .match(
              new RegExp(
                `<button[^>]*aria-label="${label}"[^>]*>([\\s\\S]*?)</button>`,
              ),
            )?.[1]
            .match(/class="voice-control-text"[^>]*>([^<]*)</)?.[1] ?? null,
      );
    };
    expect(words()).toEqual([null, null]);
    fixture.voice.muted = true;
    expect(words()).toEqual(["Stumm", null]);
    // Deafened, the headset says it for both, as a participant's state does.
    fixture.voice.deafened = true;
    expect(words()).toEqual([null, "Taub"]);
    // The fullscreen strip keeps to its icons, as the desktop app's does.
    expect(words(renderToStaticMarkup(<VoiceAudioToggles />))).toEqual([
      null,
      null,
    ]);
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
    // alice's camera on the stage; bob (muted), carol, and the own seat
    // (muted here) under it.
    expect(tiles).toEqual([
      '<li class="voice-room-participant is-speaking has-video voice-stage-item"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
      '<li class="voice-room-participant"',
    ]);
    expect(html).toContain("Spricht");
    // Alice's camera is her entry and carries the one ring; her volume is
    // a press away on it, her marks in the label with her name.
    expect(html).not.toContain("gel-avatar-lg is-speaking");
    expect(html).toContain('aria-label="Optionen für Alice"');
    expect(html).toMatch(
      /has-video voice-stage-item"(?:(?!<\/li>).)*<span class="voice-room-participant-text"><span class="voice-room-participant-marks">/,
    );
    expect(html).toContain('aria-label="Ohne Kamera"');
    expect(html).toMatch(
      /<li class="voice-room-participant is-speaking has-video voice-stage-item"(?:(?!<\/li>).)*data-source="source-from-a" data-speaking="true"/,
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

  it("shows the overview's room as its people, and keeps everyone's volume one press away", () => {
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
          overview
        />,
      );
    const empty = room();
    expect(empty).toContain('<h2 class="voice-room-name">Stage</h2>');
    // No line about who is there: an empty room is its name, one free
    // seat and the way in. The invite prompt is the channel page's.
    expect(empty).not.toContain("voice-room-summary");
    expect(empty.match(/<li class="voice-room-seat"/g)).toHaveLength(1);
    expect(empty).not.toContain("Verbunden");
    expect(empty).not.toContain("voice-room-invite");
    expect(empty).toContain("Beitreten</button>");

    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: false, deafened: false },
        bob: { channelId: "stage", muted: false, deafened: false },
      },
    };
    useUserVolumes.getState().setVolume("bob", 0.7);
    try {
      const html = room();
      expect(html).not.toContain("Alice und Bob sind da.");
      expect(html.match(/<li class="voice-room-participant"/g)).toHaveLength(2);
      // The channel's cards, compact: status line and a free seat.
      expect(html.match(/>Im Raum</g)).toHaveLength(2);
      expect(html.match(/<li class="voice-room-seat"/g)).toHaveLength(1);
      expect(html).toContain('aria-label="Optionen für Alice"');
      expect(html).toContain('aria-label="Optionen für Bob"');
      // Bob is quieter here: the card says so.
      expect(html).toMatch(
        /class="voice-room-volume-chip"[^>]*><span class="sr-only">Lautstärke <\/span>70 %/,
      );
      expect(html.match(/voice-room-volume-chip/g)).toHaveLength(1);
      expect(html).toContain('title="Lokale Lautstärke"');

      // In the call: the heading says so, and the way in is gone.
      Object.assign(fixture.voice, {
        status: "joined",
        serverId: "a",
        channelId: "stage",
      });
      const inside = room();
      expect(inside).toContain(
        '<span class="voice-room-connected">Verbunden</span>',
      );
      expect(inside).not.toContain("Beitreten");
      expect(inside).not.toContain("voice-room-seat");
    } finally {
      resetUserVolumesForTests();
    }
  });

  it("asks for company under a room nobody else is in", () => {
    fixture.live = {};
    const page = () =>
      renderToStaticMarkup(
        <VoiceRoom server={server} channelId="stage" channelName="Stage" />,
      );
    const empty = page();
    expect(empty).toMatch(
      /<div class="voice-room-invite"><p>Fehlt noch jemand\?<\/p><button[^>]*>(?:<svg.*?<\/svg>)?Einladen<\/button>/,
    );
    // Under the way in, not between the seats and it.
    expect(empty.indexOf("voice-room-invite")).toBeGreaterThan(
      empty.indexOf("voice-room-actions"),
    );

    // Alone in the call: still asking.
    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    expect(page()).toContain("Nur du bist im Kanal.");
    expect(page()).toContain('class="voice-room-invite"');

    // Someone else is here: the room has its company.
    fixture.roster = {
      a: { alice: { channelId: "stage", muted: false, deafened: false } },
    };
    expect(page()).not.toContain("voice-room-invite");
    // The dialog does not hang on the prompt: whoever its link brings in
    // must not close it under the hand that copies the link.
    expect(page()).toContain('data-invite-dialog="closed"');
  });

  it("puts the people on the stage in a call without pictures", () => {
    fixture.live = {};
    const members = ["alice", "bob"].map((id) => ({
      user_id: id,
      name: id[0]!.toUpperCase() + id.slice(1),
      avatar_url: null,
      joined_at: "",
      role: "member",
    }));
    fixture.roster = {
      a: {
        alice: { channelId: "stage", muted: false, deafened: false },
        bob: { channelId: "stage", muted: true, deafened: false },
      },
    };
    fixture.speaking = { call: "a/stage", users: { alice: true } };
    const page = () =>
      renderToStaticMarkup(
        <VoiceRoom
          server={{ ...server, members } as unknown as ServerDetail}
          channelId="stage"
          channelName="Stage"
        />,
      );

    // Before joining: the cards, as shipped.
    const outside = page();
    expect(outside).not.toContain("data-stage");
    expect(outside).toContain('aria-label="Teilnehmer im Sprachkanal"');

    Object.assign(fixture.voice, {
      status: "joined",
      serverId: "a",
      channelId: "stage",
    });
    const html = page();
    expect(html).toMatch(/<section class="voice-room"[^>]*data-stage="people"/);
    expect(html).toContain(
      '<div class="voice-people"><ul class="voice-stage" aria-label="Bühne" data-view="people">',
    );
    expect(html).not.toContain("voice-room-participants");
    // Every occupant a tile, in the roster's order, the own one too; before
    // the first measure each is small, its avatar 36 pixels.
    expect(
      html.match(/<li class="voice-room-participant[^"]*"[^>]*>/g),
    ).toEqual([
      '<li class="voice-room-participant is-speaking is-fill is-small voice-stage-item" style="--person-avatar:36px">',
      '<li class="voice-room-participant is-fill is-small voice-stage-item" style="--person-avatar:36px">',
      '<li class="voice-room-participant is-fill is-small voice-stage-item" style="--person-avatar:36px">',
    ]);
    expect(html).toContain("gel-avatar-lg is-speaking");
    expect(html).toContain(">Spricht<");
    expect(html).toContain(">Stumm<");
    expect(html).toContain(">Im Raum<");
    expect(html).toContain('<span class="voice-room-self"> (du)</span>');
    // Volume and profile stay a press away.
    expect(html).toContain('aria-label="Optionen für Alice"');
    expect(html).toContain('aria-label="Profil von Bob"');
    expect(html).not.toContain("voice-room-invite");

    // A picture brings the pictures' stage back.
    fixture.voice.participants = { alice: { pubs: ["v"] } };
    fixture.voice.remote = { alice: { v: stream } };
    const camera = page();
    expect(camera).not.toContain('data-view="people"');
    expect(camera).not.toContain("is-fill");
    expect(camera).toMatch(/<section class="voice-room"[^>]*data-stage="true"/);

    // Alone in the call: the shipped card and the invite.
    fixture.voice.participants = {};
    fixture.voice.remote = {};
    fixture.roster = {};
    const alone = page();
    expect(alone).not.toContain("data-stage");
    expect(alone).toContain('aria-label="Teilnehmer im Sprachkanal"');
    expect(alone).toContain('class="voice-room-invite"');
  });
});
