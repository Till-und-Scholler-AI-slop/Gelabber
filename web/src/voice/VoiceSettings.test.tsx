import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

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
