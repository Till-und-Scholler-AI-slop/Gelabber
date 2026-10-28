import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

// A static render keeps a hook's first state: the device list is set here.
const listing = vi.hoisted(() => ({ loaded: false }));
vi.mock("./devices.ts", async (original) => {
  const actual = await original<typeof import("./devices.ts")>();
  return {
    ...actual,
    useMediaDevices: () => ({
      devices: { audioinput: [], audiooutput: [], videoinput: [] },
      loaded: listing.loaded,
      reveal: async () => false,
    }),
  };
});

import { VoiceSettingsTabs } from "../components/VoiceSettingsDialog.tsx";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { CallSound, QuickCallSettings } from "./QuickCallSettings.tsx";
import { MediaSettingsForm } from "./VoiceSettings.tsx";
import { VoiceSettingsPanelPart } from "./voiceSettingsPanelPart.ts";

describe("stream settings form", () => {
  it("shows camera and screen profiles with resolution, fps, and budget", () => {
    const html = renderToStaticMarkup(<MediaSettingsForm />);
    expect(html).toMatch(/<h3[^>]*>Kamera<\/h3>/);
    // One set for the screen: Live is the only screen there is.
    expect(html).toMatch(/<h3[^>]*>Live<\/h3>/);
    expect(html).not.toMatch(/Go Live|Bildschirmfreigabe/);
    for (const resolution of ["480p", "720p", "1080p", "1440p", "4K · 2160p"])
      expect(html).toContain(resolution);
    for (const fps of [15, 24, 30, 45, 60])
      expect(html).toContain(`${fps} FPS`);
    expect(html).toContain('id="camera-stream-profile-resolution"');
    expect(html).toContain('id="screen-stream-profile-fps"');
    expect(html).toContain("Eigenes Limit");
    expect(html).toContain("Änderungen gelten beim nächsten Start");
  });

  it("names each setting once, in the row it heads", () => {
    const html = renderToStaticMarkup(<MediaSettingsForm section="audio" />);
    for (const [id, name] of [
      ["audio-input", "Mikrofon"],
      ["input-gain", "Mic-Gain"],
      ["output-volume", "Gespräche"],
      ["source-audio-volume", "Stream-Ton"],
      ["call-sounds", "Signaltöne"],
    ])
      expect(html).toContain(
        `<label for="${id}" class="media-row-label">${name}</label>`,
      );
    // The economy mode limits uploads; it sits with the video upload only.
    expect(html).not.toContain('id="economy-mode"');
    expect(
      renderToStaticMarkup(<MediaSettingsForm section="video" />),
    ).toContain(
      '<label for="economy-mode" class="media-row-label">Sparmodus</label>',
    );
  });
});

describe("in-call settings", () => {
  it("leave message notifications to the settings page", () => {
    const html = renderToStaticMarkup(<MediaSettingsForm />);
    expect(html).not.toContain("Nachrichten");
    expect(html).not.toContain('id="message-toasts"');
    expect(html).not.toContain('id="desktop-notify"');
  });
});

describe("source-audio form", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("explains browser-selected capture and separate listening controls", () => {
    // A desktop browser: sound for a share needs a share to begin with.
    vi.stubGlobal("navigator", {
      mediaDevices: { getDisplayMedia() {}, getUserMedia() {} },
    });
    const html = renderToStaticMarkup(<MediaSettingsForm />);
    expect(html).toContain('id="share-source-audio"');
    expect(html).toContain("Ton mitsenden");
    expect(html).toContain('aria-label="Ton im Live"');
    expect(html).toContain("im Browserdialog aus");
    expect(html).toContain('id="source-audio-volume"');
    expect(html).toContain("Gespräche");
    expect(html).toContain("Stream-Ton stummschalten");
  });
});

describe("settings dialog", () => {
  // The tab bodies are a chunk of their own; loaded, they render at once.
  beforeAll(() => VoiceSettingsPanelPart.preload());
  it("shows one area at a time, opening where it was asked to", () => {
    const tabs = (initial: "audio" | "video" | "diagnostics") =>
      renderToStaticMarkup(<VoiceSettingsTabs initial={initial} />);
    const audio = tabs("audio");
    expect(audio.match(/role="tab"/g)).toHaveLength(3);
    expect(audio).toMatch(/aria-selected="true"[^>]*>Audio</);
    expect(audio).toContain('id="audio-input"');
    expect(audio).not.toContain('id="video-input"');
    expect(audio).not.toContain("Sprachdiagnose");
    const video = tabs("video");
    expect(video).toMatch(/aria-selected="true"[^>]*>Video &amp; Stream</);
    expect(video).toContain('id="video-input"');
    expect(video).toContain('id="economy-mode"');
    expect(video).not.toContain('id="audio-input"');
    // The call bar's quality bars open the diagnostics.
    const diagnostics = tabs("diagnostics");
    expect(diagnostics).toMatch(/aria-selected="true"[^>]*>Diagnose</);
    expect(diagnostics).toContain('aria-label="Sprachdiagnose"');
    // The dialog's focus starts on the tab it opened, not the first one.
    expect(diagnostics.match(/autofocus=""/g)).toHaveLength(1);
    expect(diagnostics).toMatch(/autofocus=""[^>]*aria-selected="true"/);
  });
});

describe("quick call settings", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    listing.loaded = false;
  });

  it("keeps devices, volumes and the input mode one tap away", () => {
    vi.stubGlobal("navigator", {
      mediaDevices: { getDisplayMedia() {}, getUserMedia() {} },
    });
    const quick = () =>
      renderToStaticMarkup(<QuickCallSettings onMore={() => undefined} />);
    // Until the first listing is in, nothing asks for names.
    expect(quick()).not.toContain("Gerätenamen anzeigen");
    listing.loaded = true;
    const html = quick();
    expect(html).toContain(">Mikrofon</label>");
    expect(html).toContain(">Kamera</label>");
    expect(html).toContain(">Gespräche</label>");
    expect(html).toContain(">Stream-Ton</label>");
    expect(html).toContain('role="radiogroup" aria-label="Sprachübertragung"');
    expect(html).toContain("Alle Einstellungen …");
    // A browser that has named no device yet is asked on request only.
    expect(html).toContain("Gerätenamen anzeigen");
    // No meter outside a call: there is nothing being sent.
    expect(html).not.toContain("<meter");
  });
});

describe("call sound", () => {
  it("holds both volumes and the stream's mute", () => {
    const html = renderToStaticMarkup(<CallSound />);
    expect(html).toContain(">Gespräche</label>");
    expect(html).toContain(">Stream-Ton</label>");
    expect(html).toMatch(
      /<button[^>]*aria-pressed="false"[^>]*aria-label="Stream-Ton stummschalten"/,
    );
    expect(html).not.toContain("disabled");
    expect(html).not.toContain("Taubgeschaltet");
  });
});

describe("microphone test", () => {
  it("switches the processing in place", () => {
    const html = renderToStaticMarkup(
      <MicrophoneTest onClose={() => undefined} />,
    );
    expect(html).toContain("<legend>Verarbeitung</legend>");
    expect(html.match(/type="radio"/g)).toHaveLength(3);
    expect(html).not.toContain("beende den Test");
  });
});
