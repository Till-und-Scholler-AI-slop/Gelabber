import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

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
import { QuickCallSettings } from "./QuickCallSettings.tsx";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

describe("stream settings form", () => {
  it("shows camera and screen profiles with resolution, fps, and budget", () => {
    const html = renderToStaticMarkup(<MediaSettingsForm />);
    expect(html).toContain("Stream-Qualität");
    expect(html).toContain("Bildschirmfreigabe und Go Live");
    for (const resolution of ["480p", "720p", "1080p", "1440p", "4K · 2160p"])
      expect(html).toContain(resolution);
    for (const fps of [15, 24, 30, 45, 60])
      expect(html).toContain(`${fps} FPS`);
    expect(html).toContain('id="camera-stream-profile-resolution"');
    expect(html).toContain('id="screen-stream-profile-fps"');
    expect(html).toContain("Eigenes Limit");
    expect(html).toContain("Änderungen gelten beim nächsten Start");
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
    expect(html).toContain("Ton teilen");
    expect(html).toContain("im Browserdialog aus");
    expect(html).toContain('id="source-audio-volume"');
    expect(html).toContain("Gespräche");
    expect(html).toContain("Stream-Ton stummschalten");
  });
});

describe("settings dialog", () => {
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
    expect(video).toContain("Stream-Qualität");
    expect(video).not.toContain('id="audio-input"');
    // The call bar's quality bars open the diagnostics.
    const diagnostics = tabs("diagnostics");
    expect(diagnostics).toMatch(/aria-selected="true"[^>]*>Diagnose</);
    expect(diagnostics).toContain('aria-label="Sprachdiagnose"');
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
