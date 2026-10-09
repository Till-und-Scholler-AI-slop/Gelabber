import type { ReactNode } from "react";
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

import { VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import {
  canChooseSpeaker,
  canShareAppAudio,
  canShareScreen,
  canUseCamera,
} from "./capabilities.ts";
import { setNativeBridgeForTests, type NativeBridge } from "./native/bridge.ts";
import { loadNativeFeatures } from "./native/features.ts";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

const capture = () => {};

/** The desktop app, answering `media_info` with `info`. Its webview may or
 * may not have capture APIs of its own; they must not matter. */
async function desktopApp(info: unknown, webview: object = {}) {
  vi.stubGlobal("navigator", webview);
  setNativeBridgeForTests({
    invoke: async (command: string) =>
      command === "media_info" ? info : Promise.reject(new Error(command)),
    channel: async () => null,
  } as unknown as NativeBridge);
  await loadNativeFeatures();
}

function browser(mediaDevices: object | undefined, setSinkId: boolean) {
  setNativeBridgeForTests(null);
  vi.stubGlobal("navigator", { mediaDevices });
  vi.stubGlobal(
    "HTMLMediaElement",
    setSinkId
      ? class {
          setSinkId() {}
        }
      : class {},
  );
}

const cases = {
  /** Released 0.5.x: no feature list; WebKitGTK without capture APIs. */
  linuxApp05: () => desktopApp({ abi: 7, version: "0.5.2", platform: "linux" }),
  /** First Windows build; WebView2 has getDisplayMedia, which is not used. */
  windowsApp: () =>
    desktopApp(
      { abi: 7, version: "0.6.0", platform: "windows", features: ["camera"] },
      { mediaDevices: { getDisplayMedia: capture, getUserMedia: capture } },
    ),
  phoneBrowser: async () =>
    browser({ getUserMedia: capture, enumerateDevices: capture }, false),
  desktopBrowser: async () =>
    browser(
      {
        getUserMedia: capture,
        getDisplayMedia: capture,
        enumerateDevices: capture,
      },
      true,
    ),
};

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
    watching: false,
    playbackBlocked: false,
  } as unknown as VoiceState;
});
afterEach(() => {
  setNativeBridgeForTests(undefined);
  vi.unstubAllGlobals();
});

const answers = () => ({
  screen: canShareScreen(),
  appAudio: canShareAppAudio(),
  camera: canUseCamera(),
  speaker: canChooseSpeaker(),
});
const controls = (canGoLive = true) =>
  renderToStaticMarkup(<VoiceControls canGoLive={canGoLive} />);
/** The opening tag of the control with this label, or null. */
const control = (html: string, label: string) =>
  html.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0] ??
  null;

describe("capabilities", () => {
  it("0.5.x Linux app without a feature list: everything it always had", async () => {
    await cases.linuxApp05();
    expect(answers()).toEqual({
      screen: true,
      appAudio: true,
      camera: true,
      speaker: true,
    });
  });

  it("Windows app reporting only the camera: no share, whatever WebView2 offers", async () => {
    await cases.windowsApp();
    expect(answers()).toEqual({
      screen: false,
      appAudio: false,
      camera: true,
      speaker: true,
    });
  });

  it("phone browser: camera, but no display capture and no speaker choice", async () => {
    await cases.phoneBrowser();
    expect(answers()).toEqual({
      screen: false,
      appAudio: false,
      camera: true,
      speaker: false,
    });
  });

  it("desktop browser: all of it", async () => {
    await cases.desktopBrowser();
    expect(answers()).toEqual({
      screen: true,
      appAudio: true,
      camera: true,
      speaker: true,
    });
  });

  it("a page without mediaDevices (insecure origin) captures nothing", () => {
    browser(undefined, true);
    expect(answers()).toEqual({
      screen: false,
      appAudio: false,
      camera: false,
      speaker: true,
    });
  });

  it("an app with screen capture but no application sound shares video only", async () => {
    await desktopApp({ features: ["screen", "camera"] });
    expect(answers()).toMatchObject({ screen: true, appAudio: false });
    // Sound without a screen to share is no share either.
    await desktopApp({ features: ["app-audio", "camera"] });
    expect(answers()).toMatchObject({ screen: false, appAudio: false });
  });

  it("promises nothing before the desktop app has answered", () => {
    setNativeBridgeForTests({
      invoke: () => new Promise(() => {}),
      channel: async () => null,
    } as unknown as NativeBridge);
    expect(answers()).toEqual({
      screen: false,
      appAudio: false,
      camera: false,
      speaker: true,
    });
    const html = controls();
    expect(html).toContain('aria-label="Mikrofon aus"');
    for (const label of ["Kamera an", "Bildschirm teilen", "Go Live"])
      expect(control(html, label)).toBeNull();
  });
});

describe("capture controls", () => {
  it("0.5.x Linux app: camera, screen and Go Live as before", async () => {
    await cases.linuxApp05();
    const html = controls();
    for (const label of ["Kamera an", "Bildschirm teilen", "Go Live"])
      expect(control(html, label)).toContain('aria-pressed="false"');
    expect(html).not.toContain("aria-disabled");
  });

  it("Windows app: share and Go Live stay visible but say why they cannot start", async () => {
    await cases.windowsApp();
    const html = controls();
    expect(control(html, "Kamera an")).toContain('aria-pressed="false"');
    const share = control(html, "Bildschirm teilen");
    expect(share).toContain('aria-disabled="true"');
    expect(share).toContain(
      'title="Diese Desktop-App kann den Bildschirm nicht teilen."',
    );
    expect(share).not.toContain("aria-pressed");
    const live = control(html, "Go Live");
    expect(live).toContain('aria-disabled="true"');
    expect(live).toContain(
      'title="Diese Desktop-App kann kein Go Live starten."',
    );
    // Without the right to go live there is nothing to explain.
    expect(control(controls(false), "Go Live")).toBeNull();
  });

  it("an app without a camera says so on the camera control", async () => {
    await desktopApp({ features: ["screen"] });
    const camera = control(controls(), "Kamera an");
    expect(camera).toContain('aria-disabled="true"');
    expect(camera).toContain(
      'title="Diese Desktop-App kann keine Kamera nutzen."',
    );
  });

  it("phone browser: no share and no Go Live control at all", async () => {
    await cases.phoneBrowser();
    const html = controls();
    expect(control(html, "Kamera an")).toContain('aria-pressed="false"');
    expect(control(html, "Bildschirm teilen")).toBeNull();
    expect(control(html, "Go Live")).toBeNull();
    expect(html).not.toContain("aria-disabled");
    expect(html).toContain('aria-label="Voice-Einstellungen"');
  });

  it("desktop browser: camera, screen and Go Live as before", async () => {
    await cases.desktopBrowser();
    const html = controls();
    for (const label of ["Kamera an", "Bildschirm teilen", "Go Live"])
      expect(control(html, label)).toContain('aria-pressed="false"');
    expect(html).not.toContain("aria-disabled");
    expect(control(controls(false), "Go Live")).toBeNull();
  });

  it("always lets a running camera, share or Live be stopped", async () => {
    Object.assign(fixture.voice, { camera: true, sharing: true, live: true });
    for (const enter of [
      cases.windowsApp,
      cases.phoneBrowser,
      () => desktopApp({ features: [] }),
    ]) {
      await enter();
      const html = controls(false);
      for (const label of ["Kamera aus", "Teilen beenden", "Live beenden"])
        expect(control(html, label)).toContain('aria-pressed="true"');
      expect(html).not.toContain("aria-disabled");
    }
  });
});

describe("stream sound notice", () => {
  const notice = () => {
    fixture.voice.sourceAudio = { s: "unavailable", l: "off" };
    return renderToStaticMarkup(<VoiceSessionControls />);
  };

  it("blames the browser only in a browser", async () => {
    await cases.desktopBrowser();
    expect(notice()).toContain(
      "Der Browser hat keinen Stream-Ton freigegeben. Das Video läuft weiter.",
    );
  });

  it("blames no browser in the desktop app", async () => {
    await cases.linuxApp05();
    const html = notice();
    expect(html).toContain(
      "Der Stream-Ton konnte nicht aufgenommen werden. Das Video läuft weiter.",
    );
    expect(html).not.toContain("Browser");
  });

  it("says when the desktop app has no application sound at all", async () => {
    await desktopApp({ features: ["screen", "camera"] });
    const html = notice();
    expect(html).toContain(
      "Diese Desktop-App kann keinen Ton von Anwendungen teilen. Das Video läuft weiter.",
    );
    expect(html).not.toContain("Browser");
  });
});

describe("settings form", () => {
  const form = () => renderToStaticMarkup(<MediaSettingsForm />);

  it("desktop browser: browser wording and a speaker choice", async () => {
    await cases.desktopBrowser();
    const html = form();
    expect(html).toContain('id="audio-output"');
    expect(html).toContain("Browser-Default");
    expect(html).toContain("Browserfilter");
    expect(html).toContain("Andere Werte laufen über Web Audio.");
    expect(html).toContain(
      "Der Browser passt die tatsächliche Bitrate an die Verbindung an.",
    );
    expect(html).toContain(
      "Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke.",
    );
    expect(html).not.toContain("Systemstandard");
  });

  it("phone browser: no speaker select where setSinkId is missing", async () => {
    await cases.phoneBrowser();
    const html = form();
    expect(html).toContain('id="audio-input"');
    expect(html).not.toContain('id="audio-output"');
    expect(html).toContain(
      "Dieser Browser kann den Lautsprecher nicht wählen.",
    );
    expect(html).toContain("Nutzt die Wiedergabelautstärke.");
    expect(html).not.toContain("gewählten Lautsprecher");
    // Per section too: the notice belongs to the audio devices.
    expect(
      renderToStaticMarkup(<MediaSettingsForm section="video" />),
    ).not.toContain("Lautsprecher nicht wählen");
  });

  it.each(["linuxApp05", "windowsApp"] as const)(
    "%s: native speaker list and no browser wording",
    async (name) => {
      await cases[name]();
      const html = form();
      // The webview has no setSinkId here; the list is the native core's.
      expect(html).toContain('id="audio-output"');
      expect(html).not.toContain("Lautsprecher nicht wählen");
      expect(html.match(/Systemstandard/g)).toHaveLength(3);
      expect(html).toContain("WebRTC-Filter");
      expect(html).toContain("Echo und WebRTC-Filter");
      expect(html).toContain(
        "Die Desktop-App passt die tatsächliche Bitrate an die Verbindung an.",
      );
      expect(html).toContain("Standard-Ausgabegerät des Systems");
      expect(html).toContain(
        "Benachrichtigung, wenn Gelabber im Hintergrund ist",
      );
      expect(html).not.toMatch(/Browser|Web Audio|\bTab\b/);
    },
  );
});
