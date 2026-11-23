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
// Outside a router: the diagnostics section reads the hash, the links
// navigate.
const route = vi.hoisted(() => ({ hash: "" }));
vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<typeof import("@tanstack/react-router")>()),
  useLocation: (options?: {
    select?: (location: { hash: string }) => unknown;
  }) =>
    options?.select
      ? options.select({ hash: route.hash })
      : { hash: route.hash },
  useNavigate: () => () => Promise.resolve(),
}));

import { useLocale } from "../i18n/locale.ts";
import { SettingsAreaContent } from "../pages/SettingsAreaContent.tsx";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { CallSound, QuickCallSettings } from "./QuickCallSettings.tsx";
import { MediaSettingsForm } from "./VoiceSettings.tsx";

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
      ["input-gain", "Mikrofonverstärkung"],
      ["output-volume", "Gespräche"],
      ["source-audio-volume", "Live-Ton"],
      ["call-sounds", "Signaltöne"],
    ])
      expect(html).toContain(
        `<label for="${id}" class="media-row-label">${name}</label>`,
      );
    // The others' mute cues have a switch beside the call sounds'.
    expect(html).toMatch(
      /id="call-sounds"[\s\S]*Töne, wenn andere sich stumm schalten[\s\S]*id="other-mute-sounds"[\s\S]*id="call-sound-volume"/,
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
    expect(html).toContain("Live-Ton stummschalten");
  });
});

describe("settings → audio and video", () => {
  // #570: the one place for the call's settings; the diagnostics are a
  // section at the end of Audio, where the call bar's links lead.
  it("ends Audio with the diagnostics, and Video without them", () => {
    const audio = renderToStaticMarkup(<SettingsAreaContent area="audio" />);
    expect(audio).toContain('id="audio-input"');
    expect(audio).not.toContain('id="video-input"');
    expect(audio).toMatch(/<section id="diagnostics"[^>]*>/);
    expect(audio).toMatch(/<h3[^>]*tabindex="-1"[^>]*>Diagnose<\/h3>/);
    expect(audio).toContain('aria-label="Sprachdiagnose"');
    expect(audio.indexOf('id="audio-input"')).toBeLessThan(
      audio.indexOf('id="diagnostics"'),
    );
    const video = renderToStaticMarkup(<SettingsAreaContent area="video" />);
    expect(video).toContain('id="video-input"');
    expect(video).toContain('id="economy-mode"');
    expect(video).not.toContain('id="diagnostics"');
    expect(video).not.toContain('id="audio-input"');
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
    expect(html).toContain(">Live-Ton</label>");
    expect(html).toContain('role="radiogroup" aria-label="Sprachübertragung"');
    expect(html).toContain("Alle Audio-Einstellungen →");
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
    expect(html).toContain(">Live-Ton</label>");
    expect(html).toMatch(
      /<button[^>]*aria-pressed="false"[^>]*aria-label="Live-Ton stummschalten"/,
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
    expect(html.match(/type="radio"/g)).toHaveLength(4);
    expect(html).not.toContain("beende den Test");
  });
});

describe("in English", () => {
  afterEach(() => useLocale.setState({ locale: "de" }));

  it("names the settings, devices and limits in English", () => {
    useLocale.setState({ locale: "en" });
    const form = renderToStaticMarkup(<MediaSettingsForm />);
    expect(form).toContain("Microphone gain");
    expect(form).toContain("Enhanced voice: Standard");
    expect(form).toContain("Enhanced voice: Strong");
    expect(form).toContain("Data saver");
    // Numbers in the language's own form: 2.5, not 2,5.
    expect(form).toContain("video 2.5 Mbit/s combined");
    expect(form).toContain('<output for="input-gain">100%</output>');
    const audio = renderToStaticMarkup(<SettingsAreaContent area="audio" />);
    expect(audio).toMatch(/<h3[^>]*>Diagnostics<\/h3>/);
    const quick = renderToStaticMarkup(
      <QuickCallSettings onMore={() => undefined} />,
    );
    expect(quick).toContain(">Calls</label>");
    expect(quick).toContain("All audio settings →");
  });
});
