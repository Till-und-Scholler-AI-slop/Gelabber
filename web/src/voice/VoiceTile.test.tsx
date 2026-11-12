import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

// A static render shows a store's initial state; the call is read as it is
// set here.
vi.mock("./session.ts", async (original) => {
  const actual = await original<typeof import("./session.ts")>();
  const current = (select: (state: unknown) => unknown) =>
    select(actual.useVoice.getState());
  return { ...actual, useVoice: Object.assign(current, actual.useVoice) };
});

import { resetVoiceForTests, useVoice } from "./session.ts";
import { StreamViewer, VoiceTile } from "./VoiceTile.tsx";

/** Tag and classes of the element that holds the picture. */
function surface(html: string): { tag: string; classes: string[] } {
  const [, tag, attributes] = /<(video)([^>]*)>/.exec(html)!;
  return {
    tag,
    classes: /class="([^"]*)"/.exec(attributes)![1].split(" ").filter(Boolean),
  };
}

afterEach(() => {
  resetVoiceForTests();
});

describe("video tile in a browser", () => {
  it("plays the stream in a video element", () => {
    const browserStream = {
      getVideoTracks: () => [],
    } as unknown as MediaStream;
    const html = renderToStaticMarkup(
      <VoiceTile stream={browserStream} label="Alex" mirror />,
    );
    expect(surface(html)).toEqual({
      tag: "video",
      classes: ["size-full", "object-cover", "-scale-x-100"],
    });
    expect(html).not.toContain("<span>Alex</span>");
    // Two actions: the spotlight and the whole screen, no third "bigger".
    expect(html).toContain('aria-label="Vollbild"');
    expect(html).not.toContain("Vergrößern");
    expect(html).not.toContain("Anheften");
  });

  it("shows the name while there is no stream", () => {
    const html = renderToStaticMarkup(<VoiceTile stream={null} label="Alex" />);
    expect(surface(html).tag).toBe("video");
    expect(surface(html).classes).toContain("opacity-0");
    expect(html).toContain("<span>Alex</span>");
  });
});

describe("the own Live's sound", () => {
  const tile = (notice: string, on: boolean) =>
    renderToStaticMarkup(
      <VoiceTile
        stream={null}
        label="Lena (du) — Live"
        screen
        live
        sourceAudioNotice={notice}
        sourceAudioOn={on}
      />,
    );

  it("says it in a tag beside the Live tag, in full for screen readers", () => {
    const html = tile("Ton wird mitgesendet", true);
    expect(html).toMatch(
      /<div class="voice-video-tags"><span class="voice-video-live">Live<\/span><span role="status" class="voice-video-sound" title="Ton wird mitgesendet">/,
    );
    expect(html).toContain('<span aria-hidden="true">Mit Ton</span>');
    expect(html).toContain('<span class="sr-only">Ton wird mitgesendet</span>');
  });

  it("marks sound that does not go out", () => {
    const html = tile("Live-Ton beendet · Video läuft weiter", false);
    expect(html).toContain('class="voice-video-sound voice-video-sound--off"');
    expect(html).toContain('<span aria-hidden="true">Ohne Ton</span>');
    expect(html).toContain(
      '<span class="sr-only">Live-Ton beendet · Video läuft weiter</span>',
    );
  });

  it("has no tag without a notice", () => {
    const html = renderToStaticMarkup(
      <VoiceTile stream={null} label="Lena (du) — Live" screen live />,
    );
    expect(html).not.toContain("voice-video-sound");
  });
});

describe("tile actions", () => {
  it("pins to the spotlight with a fixed name and its state", () => {
    const tile = (pinned: boolean) =>
      renderToStaticMarkup(
        <VoiceTile
          stream={null}
          label="Mia — Bildschirm"
          screen
          pinned={pinned}
          onTogglePin={() => undefined}
        />,
      );
    expect(tile(false)).toMatch(/aria-label="Anheften" aria-pressed="false"/);
    expect(tile(true)).toMatch(/aria-label="Anheften" aria-pressed="true"/);
  });

  it("has the stream sound on a watched Live in front only", () => {
    const tile = (spotlight: boolean, watching: boolean) =>
      renderToStaticMarkup(
        <VoiceTile
          stream={null}
          label="Mia — Live"
          screen
          live
          spotlight={spotlight}
          sourceWatch={{ watching, toggle: () => undefined }}
        />,
      );
    expect(tile(true, true)).toMatch(
      /aria-label="Live-Ton stummschalten" aria-pressed="false"/,
    );
    expect(tile(false, true)).not.toContain("Live-Ton");
    expect(tile(true, false)).not.toContain("Live-Ton");
  });

  it("names a camera in a participant's place without a second caption", () => {
    const html = renderToStaticMarkup(
      <VoiceTile stream={null} label="Mia" caption={false} />,
    );
    expect(html).toContain('<figure aria-label="Mia"');
    expect(html).not.toContain("<figcaption");
  });
});

describe("watching full size", () => {
  const viewer = () =>
    renderToStaticMarkup(
      <StreamViewer
        stream={null}
        label="Mia — Live"
        screen
        live
        onStopWatching={() => undefined}
        onClose={() => undefined}
      />,
    );

  it("brings its own strip, since the call bar is out of sight", () => {
    const html = viewer();
    expect(html).toContain('aria-label="Steuerung: Mia — Live"');
    expect(html).toContain("<span>Vollbild beenden</span>");
    expect(html).toContain("<span>Nicht mehr zuschauen</span>");
    expect(html).toContain('aria-label="Live-Ton-Lautstärke"');
    expect(html).toContain('aria-label="Live-Ton stummschalten"');
    // Shown when it opens; it hides after a few idle seconds.
    expect(html).toContain("data-awake");
    // Not in a call: nothing to mute.
    expect(html).not.toContain('aria-label="Stummschalten"');
    // The tile's own buttons step aside.
    expect(html).not.toContain('aria-label="Vollbild"');
    expect(html).not.toContain('aria-label="Anheften"');
  });

  it("offers mute and deafen while in the call", () => {
    useVoice.setState({ status: "joined" });
    const html = viewer();
    expect(html).toContain('aria-label="Stummschalten"');
    expect(html).toContain('aria-label="Taubschalten"');
  });

  it("has no stream sound for a camera", () => {
    const html = renderToStaticMarkup(
      <StreamViewer stream={null} label="Mia" onClose={() => undefined} />,
    );
    expect(html).toContain("<span>Vollbild beenden</span>");
    expect(html).not.toContain("Live-Ton");
    expect(html).not.toContain("Nicht mehr zuschauen");
  });
});

describe("a Live's caption on the stage (#521)", () => {
  const live = (props: { sourceAudioNotice?: string } = {}) =>
    renderToStaticMarkup(
      <VoiceTile
        stream={null}
        label="Jonas — Live"
        screen
        live
        liveCaption={{ name: "Jonas", watchers: <b data-count="">3</b> }}
        {...props}
      />,
    );

  it("is the badge, the name and the count, with no Live tag on top", () => {
    const html = live();
    expect(html).toContain(
      '<figcaption class="voice-video-caption voice-live-caption"><span class="voice-live-caption-badge">Live</span><span class="voice-live-caption-name">Jonas</span><b data-count="">3</b></figcaption>',
    );
    expect(html).not.toContain("voice-video-live");
    expect(html).not.toContain("voice-video-tags");
  });

  it("keeps the own Live's sound tag top left", () => {
    const html = live({ sourceAudioNotice: "Ton wird mitgesendet" });
    expect(html).toMatch(
      /<div class="voice-video-tags"><span role="status" class="voice-video-sound[ "]/,
    );
    expect(html).not.toContain("voice-video-live");
  });

  it("leaves cameras and the overview's Lives as they were", () => {
    const camera = renderToStaticMarkup(
      <VoiceTile stream={null} label="Mia" />,
    );
    expect(camera).toContain(
      '<figcaption class="voice-video-caption">Mia</figcaption>',
    );
    const overview = renderToStaticMarkup(
      <VoiceTile stream={null} label="Jonas — Live" screen live />,
    );
    expect(overview).toContain(
      '<figcaption class="voice-video-caption">Jonas — Live</figcaption>',
    );
    expect(overview).toContain('<span class="voice-video-live">Live</span>');
  });
});
