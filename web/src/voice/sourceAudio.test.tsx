// The sound of the own screen share or Go Live as the user meets it: the
// switch next to the share button, the same switch in the settings, and what
// they say when sound is missing.

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
  stopWatching: vi.fn(),
  toggleCamera: vi.fn(),
  toggleDeafen: vi.fn(),
  toggleGoLive: vi.fn(),
  toggleMute: vi.fn(),
  toggleShare: vi.fn(),
}));
vi.mock("../servers/queries.ts", () => ({
  useServer: () => ({ data: { permissions: ["go_live"] } }),
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

import { VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import { setNativeBridgeForTests, type NativeBridge } from "./native/bridge.ts";
import { loadNativeFeatures } from "./native/features.ts";
import {
  resetMediaSettingsForTests,
  SOURCE_AUDIO_CARRIES_CALL,
  useMediaSettings,
  type MediaSettings,
} from "./settings.ts";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

const capture = () => {};

/** The desktop app, answering `media_info` with `info`. */
async function desktopApp(info: unknown, webview: object = {}) {
  vi.stubGlobal("navigator", webview);
  setNativeBridgeForTests({
    invoke: async (command: string) =>
      command === "media_info" ? info : Promise.reject(new Error(command)),
    channel: async () => null,
  } as unknown as NativeBridge);
  await loadNativeFeatures();
}

function browser(mediaDevices: object | undefined) {
  setNativeBridgeForTests(null);
  vi.stubGlobal("navigator", { mediaDevices });
}

const LINUX_06 = ["screen", "camera", "app-audio", "video-frames"];
const cases = {
  desktopBrowser: async () =>
    browser({ getUserMedia: capture, getDisplayMedia: capture }),
  /** No display capture on a phone, so no share and no sound for one. */
  phoneBrowser: async () => browser({ getUserMedia: capture }),
  insecureOrigin: async () => browser(undefined),
  /** Released 0.5.x: no feature list; "every application" includes itself. */
  linuxApp05: () => desktopApp({ abi: 7, version: "0.5.2", platform: "linux" }),
  /** A v0.6 build whose core still captures itself. */
  linuxApp06CapturesItself: () => desktopApp({ features: LINUX_06 }),
  linuxApp06: () =>
    desktopApp({ features: [...LINUX_06, "app-audio-excludes-self"] }),
  /** First Windows build; WebView2's own getDisplayMedia must not count. */
  windowsApp: () =>
    desktopApp(
      { features: ["camera", "video-frames"] },
      { mediaDevices: { getDisplayMedia: capture, getUserMedia: capture } },
    ),
  screenWithoutSound: () => desktopApp({ features: ["screen", "camera"] }),
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
    sourceAudio: { s: "off", l: "off" },
    sourceAudioNote: { s: null, l: null },
    sourceSubscriptions: {},
    localScreen: null,
    localLive: null,
    watching: false,
    playbackBlocked: false,
  } as unknown as VoiceState;
});
afterEach(() => {
  setNativeBridgeForTests(undefined);
  resetMediaSettingsForTests();
  vi.unstubAllGlobals();
});

const choose = (settings: Partial<MediaSettings>) =>
  useMediaSettings.getState().patch(settings);
const controls = () => renderToStaticMarkup(<VoiceControls canGoLive />);
const session = () => renderToStaticMarkup(<VoiceSessionControls />);
const form = () => renderToStaticMarkup(<MediaSettingsForm />);
const ON = "Stream-Ton nicht mehr teilen";
const OFF = "Stream-Ton teilen";
/** The opening tag of the control with this label, or null. */
const control = (html: string, label: string) =>
  html.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0] ??
  null;
/** The stream-sound switch among the call controls: on, off or absent. */
function soundControl(): "on" | "off" | null {
  const html = controls();
  const on = control(html, ON);
  const off = control(html, OFF);
  expect(on === null || off === null).toBe(true);
  if (on) expect(on).toContain('aria-pressed="true"');
  if (off) expect(off).toContain('aria-pressed="false"');
  return on ? "on" : off ? "off" : null;
}
/** The same switch in the settings form. */
function soundSetting(): "on" | "off" | null {
  const input = form().match(/<input id="share-source-audio"[^>]*>/)?.[0];
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
/** Presses the switch among the call controls. */
function pressSoundControl(): void {
  const button = propsOf(
    elements(() => VoiceControls({ canGoLive: true })),
    (props) => String(props["aria-label"] ?? "").startsWith("Stream-Ton"),
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
  it.each([
    ["desktopBrowser", "off"],
    ["linuxApp05", "off"],
    ["linuxApp06CapturesItself", "off"],
    // The only client that keeps the call out of it shares sound unasked.
    ["linuxApp06", "on"],
  ] as const)(
    "%s: next to the share button and in the settings, %s by default",
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

  it.each([
    ["desktopBrowser", "auto"],
    ["linuxApp05", "auto"],
    ["linuxApp06CapturesItself", "auto"],
    // Only here is off something the app would not do by itself.
    ["linuxApp06", "off"],
  ] as const)(
    "%s: pressed on and off again, either switch leaves %s behind",
    async (name: Case, left) => {
      await cases[name]();
      const stored = () => useMediaSettings.getState().sourceAudioShare;
      for (const flip of [
        pressSoundControl,
        () => setSoundSetting(soundSetting() !== "on"),
      ]) {
        choose({ sourceAudioShare: "auto" });
        // Where the app shares unasked, off comes first: "on" is a press too.
        if (soundControl() === "on") flip();
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

  it("an app before v0.6 keeps no off that would hold back the updated app's sound", async () => {
    await cases.linuxApp05();
    pressSoundControl();
    expect(session()).toContain(SOURCE_AUDIO_CARRIES_CALL);
    pressSoundControl();
    expect(session()).not.toContain(SOURCE_AUDIO_CARRIES_CALL);
    expect(soundControl()).toBe("off");
    // The update the warning asked for.
    await cases.linuxApp06();
    expect(soundControl()).toBe("on");
    expect(soundSetting()).toBe("on");
    // Switched off there, it stays off.
    setSoundSetting(false);
    expect(soundControl()).toBe("off");
    await cases.linuxApp06();
    expect(soundControl()).toBe("off");
  });

  it.each([
    "phoneBrowser",
    "insecureOrigin",
    "windowsApp",
    "screenWithoutSound",
  ] as const)(
    "%s: no switch where a share cannot carry sound",
    async (name: Case) => {
      await cases[name]();
      // Not even for someone who switched it on elsewhere.
      for (const sourceAudioShare of ["auto", "on"] as const) {
        choose({ sourceAudioShare });
        expect(soundControl()).toBeNull();
        expect(soundSetting()).toBeNull();
        expect(form()).not.toContain("Ton der Bildschirmfreigabe");
        expect(form()).not.toContain('id="source-audio-app"');
      }
      // Listening to someone else's stream sound is another matter.
      expect(form()).toContain('id="source-audio-volume"');
    },
  );

  it("is absent until the desktop app has said what it can do", () => {
    setNativeBridgeForTests({
      invoke: () => new Promise(() => {}),
      channel: async () => null,
    } as unknown as NativeBridge);
    expect(soundControl()).toBeNull();
    expect(soundSetting()).toBeNull();
  });

  it("sits between the share and the Go Live button", async () => {
    await cases.desktopBrowser();
    const html = controls();
    const at = (label: string) => html.indexOf(`aria-label="${label}"`);
    expect(at("Bildschirm teilen")).toBeGreaterThan(-1);
    expect(at(OFF)).toBeGreaterThan(at("Bildschirm teilen"));
    expect(at("Go Live")).toBeGreaterThan(at(OFF));
    // Stopping stays possible, and the switch stays, while a share runs.
    Object.assign(fixture.voice, { sharing: true, live: true });
    expect(soundControl()).toBe("off");
  });

  it("offers the application choice in the desktop app only", async () => {
    await cases.desktopBrowser();
    expect(form()).not.toContain('id="source-audio-app"');
    expect(form()).toContain("im Browserdialog aus");
    await cases.linuxApp06();
    const html = form();
    expect(html).toContain('id="source-audio-app"');
    expect(html).toContain("Alle Anwendungen außer Gelabber");
    expect(html).toContain("nie dein Mikrofon oder der Ton des Anrufs");
    expect(html).toContain("auch während sie laufen");
    expect(html).not.toMatch(/Browser/);
  });
});

describe("stream sound that also carries the call", () => {
  const warned = () => ({
    session: session().includes(SOURCE_AUDIO_CARRIES_CALL),
    settings: form().includes(SOURCE_AUDIO_CARRIES_CALL),
  });
  const both = (value: boolean) => ({ session: value, settings: value });

  it.each(["linuxApp05", "linuxApp06CapturesItself"] as const)(
    "%s: says so once every application is switched on",
    async (name: Case) => {
      await cases[name]();
      expect(warned()).toEqual(both(false));
      choose({ sourceAudioShare: "on" });
      expect(warned()).toEqual(both(true));
      // One short sentence: what happens, and what helps.
      expect(SOURCE_AUDIO_CARRIES_CALL).toMatch(/Ton des Anrufs.*Update/);
      expect(SOURCE_AUDIO_CARRIES_CALL).not.toMatch(/Browser/);
      // A single application is not the call.
      choose({ sourceAudioApp: "firefox" });
      expect(warned()).toEqual(both(false));
      choose({ sourceAudioApp: "", sourceAudioShare: "off" });
      expect(warned()).toEqual(both(false));
    },
  );

  it("promises a call-free share only to an app that leaves itself out", async () => {
    await cases.linuxApp05();
    expect(form()).not.toContain("Ton des Anrufs");
    expect(form()).toContain("nie dein Mikrofon.");
    await cases.linuxApp06();
    expect(form()).toContain("nie dein Mikrofon oder der Ton des Anrufs");
  });

  it.each(["linuxApp06", "desktopBrowser"] as const)(
    "%s: nothing to warn about",
    async (name: Case) => {
      await cases[name]();
      choose({ sourceAudioShare: "on" });
      expect(warned()).toEqual(both(false));
    },
  );

  it("is no topic outside a call", async () => {
    await cases.linuxApp05();
    choose({ sourceAudioShare: "on" });
    Object.assign(fixture.voice, { status: "idle", watching: true });
    expect(session()).not.toContain(SOURCE_AUDIO_CARRIES_CALL);
  });
});

describe("why a share has no sound", () => {
  it("gives the desktop app's own reason", async () => {
    await cases.linuxApp06();
    fixture.voice.sourceAudio = { s: "unavailable", l: "off" };
    fixture.voice.sourceAudioNote = {
      s: { failed: "cannot connect to PipeWire" },
      l: null,
    };
    const html = session();
    expect(html).toContain(
      "Der Stream-Ton konnte nicht aufgenommen werden (cannot connect to PipeWire). Das Video läuft weiter.",
    );
    expect(html).not.toContain("Browser");
    // Go Live reports the same way.
    fixture.voice.sourceAudio = { s: "off", l: "unavailable" };
    fixture.voice.sourceAudioNote = { s: null, l: { failed: "no sound" } };
    expect(session()).toContain("aufgenommen werden (no sound).");
  });

  it("names the chosen application that plays nothing", async () => {
    await cases.linuxApp06();
    fixture.voice.sourceAudio = { s: "sharing", l: "off" };
    expect(session()).not.toContain("kommt gerade kein Ton");
    fixture.voice.sourceAudioNote = { s: { silent: "spotify" }, l: null };
    const html = session();
    expect(html).toContain("Von „spotify“ kommt gerade kein Ton.");
    expect(html).not.toContain("Das Video läuft weiter");
  });

  it("browser: sound switched on during a share comes with the next one", async () => {
    const note = "Der Stream-Ton kommt mit der nächsten Freigabe dazu.";
    await cases.desktopBrowser();
    const running = { id: "screen" } as MediaStream;
    Object.assign(fixture.voice, { sharing: true, localScreen: running });
    expect(session()).not.toContain(note);
    choose({ sourceAudioShare: "on" });
    expect(session()).toContain(note);
    // A share that has its sound, or lost it, says that instead.
    fixture.voice.sourceAudio = { s: "sharing", l: "off" };
    expect(session()).not.toContain(note);
    fixture.voice.sourceAudio = { s: "unavailable", l: "off" };
    expect(session()).not.toContain(note);
    // Go Live alike; a picker that is still open is no running share.
    fixture.voice.sourceAudio = { s: "off", l: "off" };
    Object.assign(fixture.voice, { localScreen: null, live: true });
    expect(session()).not.toContain(note);
    Object.assign(fixture.voice, { localLive: running });
    expect(session()).toContain(note);
  });

  it("desktop app: nothing waits for the next share", async () => {
    await cases.linuxApp06();
    Object.assign(fixture.voice, {
      sharing: true,
      localScreen: { id: "screen" } as MediaStream,
    });
    expect(session()).not.toContain("nächsten Freigabe");
    expect(form()).not.toContain("nächste Bildschirmfreigabe");
  });
});
