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
  toggleCamera: vi.fn(),
  toggleDeafen: vi.fn(),
  toggleMute: vi.fn(),
  toggleShare: vi.fn(),
  retryMicrophone: vi.fn(),
  joinVoice: vi.fn(),
  dismissLostCall: vi.fn(),
}));
vi.mock("../servers/queries.ts", () => ({
  useServer: () => ({ data: { permissions: ["go_live"] } }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));

import { LiveMenuItems, VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceSessionControls } from "../components/VoiceSessionControls.tsx";
import {
  canChooseSpeaker,
  canShareAppAudio,
  canShareScreen,
  canUseCamera,
} from "./capabilities.ts";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

const capture = () => {};

function browser(mediaDevices: object | undefined, setSinkId: boolean) {
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
    sourceAudio: { s: "off" },
    sourceSubscriptions: {},
    playbackBlocked: false,
  } as unknown as VoiceState;
});
afterEach(() => {
  vi.unstubAllGlobals();
});

const answers = () => ({
  screen: canShareScreen(),
  appAudio: canShareAppAudio(),
  camera: canUseCamera(),
  speaker: canChooseSpeaker(),
});
const controls = (canLive = true) =>
  renderToStaticMarkup(<VoiceControls canLive={canLive} />);
/** The Live menu's entries, as the menu shows them once opened. */
const menu = (canLive = true) =>
  renderToStaticMarkup(<LiveMenuItems canLive={canLive} />);
/** The opening tag of the control with this label, or null. */
const control = (html: string, label: string) =>
  html.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0] ??
  null;
/** The opening tag of the menu entry with this label, or null. */
const entry = (html: string, label: string) =>
  html
    .match(
      new RegExp(
        `<button[^>]*role="menuitem"[^>]*>(?:(?!</button>).)*?<strong>${label}</strong>`,
      ),
    )?.[0]
    .match(/^<button[^>]*>/)?.[0] ?? null;

describe("capabilities", () => {
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
});

describe("capture controls", () => {
  it("phone browser: no Live control at all", async () => {
    await cases.phoneBrowser();
    const html = controls();
    expect(control(html, "Kamera")).toContain('aria-pressed="false"');
    expect(control(html, "Live gehen")).toBeNull();
    expect(html).not.toContain("aria-disabled");
    expect(html).toContain('aria-label="Voice-Einstellungen"');
  });

  it("desktop browser: camera and Live, no Go Live, no Teilen", async () => {
    await cases.desktopBrowser();
    const html = controls();
    for (const label of ["Kamera", "Live gehen"])
      expect(control(html, label)).toContain('aria-pressed="false"');
    expect(entry(menu(), "Live gehen")).not.toContain("aria-disabled");
    expect(html + menu()).not.toMatch(/Go Live|Teilen|teilen|Bildschirm/);
  });

  it("without go_live the Live entry is dimmed and says why", async () => {
    await cases.desktopBrowser();
    const html = menu(false);
    expect(entry(html, "Live gehen")).toContain('aria-disabled="true"');
    expect(html).toContain("In diesem Server darfst du nicht live gehen.");
    // The sound choice stays, for when the right comes.
    expect(html).toContain('aria-label="Ton mitsenden"');
    expect(control(controls(false), "Live gehen")).toContain("opacity-50");
  });

  it("always lets a running camera or Live be stopped", async () => {
    Object.assign(fixture.voice, { camera: true, sharing: true });
    for (const enter of [cases.phoneBrowser, cases.desktopBrowser]) {
      await enter();
      const html = controls(false);
      for (const label of ["Kamera", "Live gehen"])
        expect(control(html, label)).toContain('aria-pressed="true"');
      expect(entry(menu(false), "Live beenden")).not.toContain("aria-disabled");
      expect(html).not.toContain("aria-disabled");
    }
  });
});

describe("stream sound notice", () => {
  const notice = () => {
    fixture.voice.sourceAudio = { s: "unavailable" };
    return renderToStaticMarkup(<VoiceSessionControls />);
  };

  it("blames the browser", async () => {
    await cases.desktopBrowser();
    expect(notice()).toContain(
      "Der Browser hat keinen Stream-Ton freigegeben. Das Video läuft weiter.",
    );
  });
});

describe("settings form", () => {
  const form = () => renderToStaticMarkup(<MediaSettingsForm />);

  it("desktop browser: browser wording and a speaker choice", async () => {
    await cases.desktopBrowser();
    const html = form();
    expect(html).toContain('id="audio-output"');
    // The system's choice; it names the device once the browser lists one.
    expect(html).toContain('selected="">Standard</option>');
    // Asking for names waits for the first listing.
    expect(html).not.toContain("Gerätenamen anzeigen");
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
});
