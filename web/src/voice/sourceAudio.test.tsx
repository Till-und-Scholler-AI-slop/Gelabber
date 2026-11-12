// The sound of the own Live as the user meets it: the switch in the Live
// menu, the same switch in the settings, and what the call bar and the own
// tile say when sound is missing.

import { createElement, isValidElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VoiceState } from "./session.ts";

const fixture = vi.hoisted(() => ({ voice: {} as VoiceState }));
vi.mock("./session.ts", () => ({
  useVoice: (select?: (state: VoiceState) => unknown) =>
    select ? select(fixture.voice) : fixture.voice,
  leaveVoice: vi.fn(),
  retryPlayback: vi.fn(),
  toggleCamera: vi.fn(),
  toggleDeafen: vi.fn(),
  toggleMute: vi.fn(),
  toggleShare: vi.fn(),
  joinVoice: vi.fn(),
  toggleSourceWatch: vi.fn(),
  retryMicrophone: vi.fn(),
  dismissLostCall: vi.fn(),
}));
vi.mock("../servers/queries.ts", () => ({
  useServer: () => ({ data: { permissions: ["go_live"] } }),
}));
// VoiceRoom reads the user through session.ts, useUserId through store.ts,
// so both hand out the same one.
const signedIn = vi.hoisted(() => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "self", name: "Rafi", avatar_url: null } }),
}));
vi.mock("../auth/session.ts", () => signedIn);
vi.mock("../auth/store.ts", () => signedIn);
vi.mock("./roster.ts", () => ({
  EMPTY_OCCUPANCY: {},
  EMPTY_LIVE_PEOPLE: {},
  EMPTY_WATCHES: {},
  useVoiceRoster: (select: (state: unknown) => unknown) =>
    select({ byServer: {}, live: {}, watches: {} }),
}));
// A tile as its label and what it says about the sound.
vi.mock("./VoiceTile.tsx", () => ({
  VoiceTile: ({
    label,
    sourceAudioNotice,
  }: {
    label: string;
    sourceAudioNotice?: string;
  }) => <span data-tile={label}>{sourceAudioNotice}</span>,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));
// A static render shows a store's initial state. These tests are about the
// choices made after it, so the hook reads the store as it is.
vi.mock("./settings.ts", async (original) => {
  const actual = await original<typeof import("./settings.ts")>();
  const current = (select?: (state: unknown) => unknown) =>
    select
      ? select(actual.useMediaSettings.getState())
      : actual.useMediaSettings.getState();
  return {
    ...actual,
    useMediaSettings: Object.assign(current, actual.useMediaSettings),
  };
});

import {
  LiveMenuItems,
  ShareSoundSwitch,
} from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import { dockNotices } from "./callBar.ts";
import {
  resetMediaSettingsForTests,
  useMediaSettings,
  type MediaSettings,
} from "./settings.ts";
import { MediaSettingsForm } from "./VoiceSettings.tsx";
import { VoiceRoom } from "./VoiceRoom.tsx";
import type { ServerDetail } from "../servers/types.ts";

const capture = () => {};

function browser(mediaDevices: object | undefined) {
  vi.stubGlobal("navigator", { mediaDevices });
}

const cases = {
  desktopBrowser: async () =>
    browser({ getUserMedia: capture, getDisplayMedia: capture }),
  /** No display capture on a phone, so no share and no sound for one. */
  phoneBrowser: async () => browser({ getUserMedia: capture }),
  insecureOrigin: async () => browser(undefined),
};
type Case = keyof typeof cases;

beforeEach(() => {
  fixture.voice = {
    status: "joined",
    serverId: "a",
    channelId: "stage",
    channelName: "Stage",
    muted: false,
    deafened: false,
    camera: false,
    sharing: false,
    live: false,
    sourceAudio: { s: "off" },
    sourceSubscriptions: {},
    localScreen: null,
    participants: {},
    remote: {},
    playbackBlocked: false,
  } as unknown as VoiceState;
});
afterEach(() => {
  resetMediaSettingsForTests();
  vi.unstubAllGlobals();
});

const choose = (settings: Partial<MediaSettings>) =>
  useMediaSettings.getState().patch(settings);
const menu = () => renderToStaticMarkup(<LiveMenuItems canLive />);
const session = () => renderToStaticMarkup(<VoiceSessionControls />);
const form = () => renderToStaticMarkup(<MediaSettingsForm />);
/** The "Ton mitsenden" entry of the Live menu, or null. */
const soundEntry = (html: string) =>
  html.match(
    /<button[^>]*role="menuitemcheckbox"[^>]*>(?:(?!<\/button>).)*<\/button>/,
  )?.[0] ?? null;
/** What the entry says about whose sound a share carries. */
function soundControlHint(): string | null {
  const entry = soundEntry(menu());
  expect(entry).toContain("<strong>Ton mitsenden</strong>");
  return entry?.match(/<small[^>]*>([^<]*)<\/small>/)?.[1] ?? null;
}
/** The stream-sound switch in the Live menu: on, off or absent. */
function soundControl(): "on" | "off" | null {
  const entry = soundEntry(menu());
  if (!entry) return null;
  return entry.includes('aria-checked="true"') ? "on" : "off";
}
/** The same switch in the settings form. */
function soundSetting(): "on" | "off" | null {
  const input = form().match(/<input[^>]*id="share-source-audio"[^>]*>/)?.[0];
  if (!input) return null;
  return input.includes("checked") ? "on" : "off";
}

/** What a component hands to React, handlers included: a static render
 * leaves them out. */
function elements(component: () => ReactNode): ReactNode {
  let out: ReactNode = null;
  const probe = () => {
    out = component();
    return null;
  };
  renderToStaticMarkup(createElement(probe));
  return out;
}
type Props = Record<string, unknown>;
function propsOf(node: ReactNode, wanted: (props: Props) => boolean): Props {
  const search = (at: ReactNode): Props | null => {
    if (Array.isArray(at)) {
      for (const child of at) {
        const found = search(child);
        if (found) return found;
      }
      return null;
    }
    if (!isValidElement(at)) return null;
    const props = at.props as Props;
    return wanted(props) ? props : search(props.children as ReactNode);
  };
  const found = search(node);
  if (!found) throw new Error("no such element");
  return found;
}
/** Presses the switch in the Live menu. */
function pressSoundControl(): void {
  const entry = propsOf(
    elements(() => LiveMenuItems({ canLive: true })),
    (props) => props.role === "menuitemcheckbox",
  );
  // The entry is the switch's own component: what that hands to React.
  const button = propsOf(
    elements(() =>
      ShareSoundSwitch(entry as Parameters<typeof ShareSoundSwitch>[0]),
    ),
    (props) => props.role === "menuitemcheckbox",
  );
  (button.onClick as () => void)();
}
/** Sets the switch in the settings form. */
function setSoundSetting(on: boolean): void {
  const toggle = propsOf(
    elements(() => MediaSettingsForm({})),
    (props) => props.id === "share-source-audio",
  );
  (toggle.onChange as (on: boolean) => void)(on);
}

describe("stream sound switch", () => {
  it.each([["desktopBrowser", "off"]] as const)(
    "%s: in the Live menu and in the settings, %s by default",
    async (name: Case, byDefault) => {
      await cases[name]();
      expect(soundControl()).toBe(byDefault);
      expect(soundSetting()).toBe(byDefault);
      choose({ sourceAudioShare: "on" });
      expect(soundControl()).toBe("on");
      expect(soundSetting()).toBe("on");
      choose({ sourceAudioShare: "off" });
      expect(soundControl()).toBe("off");
      expect(soundSetting()).toBe("off");
    },
  );

  it.each([["desktopBrowser", "auto"]] as const)(
    "%s: pressed on and off again, either switch leaves %s behind",
    async (name: Case, left) => {
      await cases[name]();
      const stored = () => useMediaSettings.getState().sourceAudioShare;
      for (const flip of [
        pressSoundControl,
        () => setSoundSetting(soundSetting() !== "on"),
      ]) {
        choose({ sourceAudioShare: "auto" });
        expect(soundControl()).toBe("off");
        flip();
        expect(stored()).toBe("on");
        expect(soundControl()).toBe("on");
        expect(soundSetting()).toBe("on");
        flip();
        expect(stored()).toBe(left);
        expect(soundControl()).toBe("off");
        expect(soundSetting()).toBe("off");
      }
    },
  );

  it.each(["phoneBrowser", "insecureOrigin"] as const)(
    "%s: no switch where a Live cannot carry sound",
    async (name: Case) => {
      await cases[name]();
      // Not even for someone who switched it on elsewhere.
      for (const sourceAudioShare of ["auto", "on"] as const) {
        choose({ sourceAudioShare });
        expect(soundControl()).toBeNull();
        expect(soundSetting()).toBeNull();
        expect(form()).not.toContain("Ton im Live");
      }
      // Listening to someone else's stream sound is another matter.
      expect(form()).toContain('id="source-audio-volume"');
    },
  );

  it("leaves the choice of sound to the browser's picker", async () => {
    await cases.desktopBrowser();
    expect(soundControlHint()).toBe(
      "Wähle den Ton im Freigabedialog des Browsers aus.",
    );
    expect(form()).toContain("im Browserdialog aus");
  });

  it("follows Live gehen in the Live menu", async () => {
    await cases.desktopBrowser();
    const html = menu();
    const at = (label: string) => html.indexOf(`<strong>${label}</strong>`);
    expect(at("Live gehen")).toBe(html.indexOf("<strong>"));
    expect(at("Ton mitsenden")).toBeGreaterThan(at("Live gehen"));
    // Stopping stays possible, and the switch stays, while a Live runs.
    Object.assign(fixture.voice, { sharing: true });
    expect(soundControl()).toBe("off");
    expect(menu().indexOf("<strong>Live beenden</strong>")).toBe(
      menu().indexOf("<strong>"),
    );
  });
});

describe("whose sound a running share sends", () => {
  const running = { id: "capture" } as MediaStream;

  it("browser: its picker has said what it hands over", async () => {
    await cases.desktopBrowser();
    choose({ sourceAudioShare: "on" });
    Object.assign(fixture.voice, { sharing: true, localScreen: running });
    fixture.voice.sourceAudio = { s: "sharing" };
    expect(session()).not.toContain("Dein Stream sendet");
  });
});

describe("why a share has no sound", () => {
  it("says one thing at a time, the most important first", async () => {
    await cases.desktopBrowser();
    fixture.voice.sourceAudio = { s: "unavailable" };
    const html = session();
    expect(html).toContain("Der Browser hat keinen Live-Ton freigegeben.");
    expect(html.match(/voice-session-notice/g)).toHaveLength(1);
    expect(
      dockNotices(
        { sourceAudio: { s: "ended" }, localScreen: null },
        { sharesSound: true },
      ).map((notice) => notice.text),
    ).toEqual(["Der Live-Ton wurde beendet. Das Video läuft weiter."]);
    // The call bar speaks of Live, never of a share.
    expect(
      dockNotices(
        {
          sourceAudio: { s: "off" },
          localScreen: { id: "screen" } as MediaStream,
        },
        { sharesSound: true },
      ).map((notice) => notice.text),
    ).toEqual(["Ton ab dem nächsten Live."]);
  });

  it("browser: sound switched on during a Live comes with the next one", async () => {
    const note = "Ton ab dem nächsten Live.";
    await cases.desktopBrowser();
    const running = { id: "screen" } as MediaStream;
    Object.assign(fixture.voice, { sharing: true, localScreen: running });
    expect(session()).not.toContain(note);
    choose({ sourceAudioShare: "on" });
    expect(session()).toContain(note);
    // A share that has its sound, or lost it, says that instead.
    fixture.voice.sourceAudio = { s: "sharing" };
    expect(session()).not.toContain(note);
    fixture.voice.sourceAudio = { s: "unavailable" };
    expect(session()).not.toContain(note);
    // A picker that is still open is no running Live.
    fixture.voice.sourceAudio = { s: "off" };
    Object.assign(fixture.voice, { localScreen: null });
    expect(session()).not.toContain(note);
  });
});

describe("the own Live's tile", () => {
  const server = {
    id: "a",
    role: "member",
    permissions: ["join_voice", "go_live"],
    members: [],
  } as unknown as ServerDetail;
  /** What the tile with this label says about its sound. */
  function tile(label: string): string | null {
    const html = renderToStaticMarkup(
      <VoiceRoom server={server} channelId="stage" channelName="Stage" />,
    );
    return (
      html.match(new RegExp(`<span data-tile="${label}">(.*?)</span>`))?.[1] ??
      null
    );
  }
  const LIVE = "Rafi \\(du\\) — Live";
  const running = { id: "capture" } as MediaStream;

  it("browser: a picker gave no sound", async () => {
    await cases.desktopBrowser();
    Object.assign(fixture.voice, { sharing: true, localScreen: running });
    expect(tile(LIVE)).toBe("");
    fixture.voice.sourceAudio = { s: "sharing" };
    expect(tile(LIVE)).toBe("Ton wird mitgesendet");
    fixture.voice.sourceAudio = { s: "unavailable" };
    expect(tile(LIVE)).toBe("Kein Live-Ton freigegeben · Video läuft weiter");
  });
});
