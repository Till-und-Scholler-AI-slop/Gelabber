import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { MediaSettingsForm } from "./VoiceSettings.tsx";

describe("stream settings form", () => {
  it("shows camera and screen profiles with resolution, fps, and budget", () => {
    const html = renderToStaticMarkup(<MediaSettingsForm />);
    expect(html).toContain("Stream-Qualität");
    expect(html).toContain("Bildschirmfreigabe und Go Live");
    expect(html).toContain("Sparsam");
    expect(html).toContain("480p · 15 FPS · max. 0,8 Mbit/s");
    expect(html).toContain("720p ideal, max. 1080p · 30 FPS · max. 2,5 Mbit/s");
    expect(html).toContain(
      "max. 1080p · 15 FPS ideal, max. 30 · max. 2,5 Mbit/s",
    );
    expect(html).toContain("1080p · 30 FPS · max. 4 Mbit/s");
    expect(html).toContain('name="camera-stream-profile"');
    expect(html).toContain('name="screen-stream-profile"');
    expect(html).toContain(
      "Audio-Bitrate, Mute und Deafen bleiben unverändert",
    );
    expect(html).toContain("Gilt beim nächsten Start");
    expect(html).toContain('name="camera-stream-profile" checked=""');
  });
});
