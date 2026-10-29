import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setNativeBridgeForTests } from "./native/bridge.ts";
import { NativeStream, NativeTrack } from "./native/tracks.ts";
import type { NativeVideoLimit } from "./native/videoFeed.ts";
import {
  closeNativeViewer,
  openNativeViewer,
  resetNativeViewersForTests,
} from "./native/viewer.ts";

// What the app around the page says it can do, and how in-page video fares.
const app = vi.hoisted(() => ({
  features: [] as string[],
  limit: null as NativeVideoLimit | null,
}));
vi.mock("./native/features.ts", () => ({
  hasNativeFeature: (name: string) => app.features.includes(name),
  subscribeNativeFeatures: () => () => undefined,
}));
vi.mock("./native/videoFeed.ts", async (original) => ({
  ...(await original<typeof import("./native/videoFeed.ts")>()),
  nativeVideoLimit: () => app.limit,
}));

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

const stream = (handle: { source?: number; consumer?: number }) =>
  new NativeStream([
    new NativeTrack("video", "x", handle) as unknown as MediaStreamTrack,
  ]) as unknown as MediaStream;

/** Tag and classes of the element that holds the picture. */
function surface(html: string): { tag: string; classes: string[] } {
  const [, tag, attributes] = /<(video|canvas)([^>]*)>/.exec(html)!;
  return {
    tag,
    classes: /class="([^"]*)"/.exec(attributes)![1].split(" ").filter(Boolean),
  };
}

/** Opens the viewer window of `watched`'s stream, in an app that answers
 * every command. */
async function openViewer(watched: MediaStream): Promise<void> {
  setNativeBridgeForTests({
    invoke: async <T,>() => null as T,
    channel: async () => ({}),
  });
  const [track] = watched.getVideoTracks() as unknown as NativeTrack[];
  await openNativeViewer(track, "Alex – Gelabber");
}

/** Until the app has answered what the page asked of it. */
const answered = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
  app.features = [];
  app.limit = null;
});
afterEach(() => {
  resetNativeViewersForTests();
  setNativeBridgeForTests(undefined);
  resetVoiceForTests();
});

describe("video tile in a browser", () => {
  it("plays the stream in a video element", () => {
    // A browser's own stream; the app's features do not matter for it.
    app.features = ["video-frames"];
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
    expect(html).not.toContain("<canvas");
    expect(html).not.toContain("<span>Alex</span>");
    expect(html).not.toContain("Fenster");
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
    expect(html).not.toContain("Keine Vorschau");
  });
});

describe("video tile in a desktop app up to 0.5.x", () => {
  it("has no picture of the own camera or screen", () => {
    for (const screen of [false, true]) {
      const html = renderToStaticMarkup(
        <VoiceTile
          stream={stream({ source: 7 })}
          label="Rafi (du)"
          mirror={!screen}
          screen={screen}
        />,
      );
      expect(html).not.toContain("<canvas");
      expect(surface(html).tag).toBe("video");
      expect(surface(html).classes).toContain("opacity-0");
      expect(html).toContain(
        '<span>Rafi (du)</span><span role="status" class="text-xs">Keine Vorschau in der Desktop-App.</span>',
      );
      expect(html).not.toContain("Fenster");
    }
  });

  it("offers a watched stream in the viewer window only", () => {
    const html = renderToStaticMarkup(
      <VoiceTile
        stream={stream({ consumer: 42 })}
        label="Alex — Live"
        screen
        live
      />,
    );
    expect(html).not.toContain("<canvas");
    expect(surface(html).classes).toContain("opacity-0");
    expect(html).toContain(
      '<button type="button" class="ui-button ui-button--primary ui-button--sm">Im Fenster ansehen</button>',
    );
    expect(html).not.toContain("Eigenes Fenster");
    expect(html).not.toContain("Keine Vorschau");
    expect(html).not.toContain("Geringe Bildqualität");
  });

  it("ignores features it does not know", () => {
    app.features = ["screen-audio"];
    const html = renderToStaticMarkup(
      <VoiceTile stream={stream({ consumer: 42 })} label="Alex" />,
    );
    expect(html).not.toContain("<canvas");
    expect(html).toContain("Im Fenster ansehen");
  });

  it("has the tile close the viewer window it opened", async () => {
    const watched = stream({ consumer: 42 });
    await openViewer(watched);
    const html = renderToStaticMarkup(
      <VoiceTile stream={watched} label="Alex" screen />,
    );
    expect(html).toContain(
      '<span>Alex</span><button type="button" class="ui-button ui-button--primary ui-button--sm">Fenster schließen</button></div>',
    );
    expect(html).not.toContain("<canvas");
    expect(html).not.toContain("Läuft im eigenen Fenster");
  });
});

describe("video tile in a desktop app with in-page video", () => {
  beforeEach(() => {
    app.features = ["video-frames"];
  });

  it("draws the own camera, mirrored, where the browser has its video", () => {
    const own = stream({ source: 7 });
    const html = renderToStaticMarkup(
      <VoiceTile stream={own} label="Rafi (du)" mirror />,
    );
    const id = own.getVideoTracks()[0]!.id;
    expect(html).toContain(`<canvas data-native-track="${id}"`);
    // The video's classes; hidden until the first frame is on it.
    expect(surface(html)).toEqual({
      tag: "canvas",
      classes: ["size-full", "object-cover", "-scale-x-100", "opacity-0"],
    });
    expect(html).not.toContain("<video");
    expect(html).not.toContain("Keine Vorschau");
    // Until then the name shows, like on a tile without a stream.
    expect(html).toContain("<span>Rafi (du)</span></div>");
    // The viewer window is for other people's streams.
    expect(html).not.toContain("Fenster");
    expect(html).toContain('aria-label="Vollbild"');
  });

  it("draws the own screen share and Live unmirrored and uncropped", () => {
    const html = renderToStaticMarkup(
      <VoiceTile
        stream={stream({ source: 9 })}
        label="Rafi — Live"
        screen
        live
      />,
    );
    expect(surface(html)).toEqual({
      tag: "canvas",
      classes: ["size-full", "object-contain", "opacity-0"],
    });
    expect(html).not.toContain("Keine Vorschau");
    expect(html).not.toContain("Fenster");
    expect(html).toContain(">Live</span>");
  });

  it("draws a watched stream in the tile and offers a window of its own", () => {
    const html = renderToStaticMarkup(
      <VoiceTile
        stream={stream({ consumer: 42 })}
        label="Alex — Live"
        screen
        onTogglePin={() => undefined}
        sourceWatch={{ watching: true, toggle: () => undefined }}
      />,
    );
    expect(surface(html)).toEqual({
      tag: "canvas",
      classes: ["size-full", "object-contain", "opacity-0"],
    });
    expect(html).not.toContain("Im Fenster ansehen");
    // Icon actions, each named; the window waits behind "…".
    const actions = /<div class="voice-video-actions">(.*?)<\/div>/.exec(
      html,
    )![1];
    expect(
      [...actions.matchAll(/<button[^>]*aria-label="([^"]*)"/g)].map(
        (match) => match[1],
      ),
    ).toEqual([
      "Nicht mehr zuschauen",
      "Anheften",
      "Vollbild",
      "Weitere Aktionen",
    ]);
    expect(actions).toMatch(/aria-label="Anheften" aria-pressed="false"/);
    expect(actions).toContain('aria-haspopup="menu"');
    expect(actions).not.toMatch(/>\s*[A-Za-zÄÖÜäöüß]/);
  });

  it("leaves a stream to its viewer window", async () => {
    const watched = stream({ consumer: 42 });
    const tile = (of: MediaStream, label: string) =>
      renderToStaticMarkup(<VoiceTile stream={of} label={label} screen live />);
    app.limit = "transport";
    await openViewer(watched);
    const html = tile(watched, "Alex — Live");
    // No canvas that would ask the app for frames: next to a window they
    // are the window's. The tile says where the stream is.
    expect(html).not.toContain("<canvas");
    expect(surface(html)).toEqual({
      tag: "video",
      classes: ["size-full", "object-contain", "opacity-0"],
    });
    expect(html).toContain(
      '<span>Alex — Live</span><span role="status" class="text-xs">Läuft im eigenen Fenster.</span><button',
    );
    // Where the picture would be, the way to close the window.
    expect(html).toContain(">Fenster schließen</button></div>");
    expect(html).not.toContain("Im Fenster ansehen");
    expect(html).not.toContain("Geringe Bildqualität");
    // Another stream, and the own source with the consumer's number.
    expect(tile(stream({ consumer: 43 }), "Kim")).toContain("<canvas");
    const own = tile(stream({ source: 42 }), "Rafi — Live");
    expect(own).toContain("<canvas");
    expect(own).not.toContain("Fenster");

    // Closed: the tile waits for the app to let go of the window.
    closeNativeViewer(42);
    const closing = tile(watched, "Alex — Live");
    expect(closing).not.toContain("<canvas");
    expect(closing).not.toContain("Fenster schließen");
    await answered();
    const back = tile(watched, "Alex — Live");
    expect(back).toContain("<canvas");
    expect(back).toContain("Geringe Bildqualität");
    expect(back).not.toContain("Läuft im eigenen Fenster");
  });

  it("keeps the placeholder for a stream without live video", () => {
    const ended = new NativeTrack("video", "x", { consumer: 42 });
    ended.stop();
    const html = renderToStaticMarkup(
      <VoiceTile
        stream={
          new NativeStream([
            ended as unknown as MediaStreamTrack,
          ]) as unknown as MediaStream
        }
        label="Alex"
      />,
    );
    expect(html).not.toContain("<canvas");
    expect(html).toContain("<span>Alex</span></div>");
    expect(html).not.toContain("Fenster");
    expect(html).not.toContain("Keine Vorschau");
  });

  it("says when the picture has to stay small", () => {
    const tile = (live: boolean) =>
      renderToStaticMarkup(
        <VoiceTile
          stream={stream({ consumer: 42 })}
          label="Alex"
          live={live}
        />,
      );
    expect(tile(false)).not.toContain("Geringe Bildqualität");
    app.limit = "transport";
    expect(tile(false)).toMatch(
      /<span role="status" title="Der Server blockiert[^"]*" class="[^"]* top-2">Geringe Bildqualität<\/span>/,
    );
    // Below the Live badge.
    expect(tile(true)).toMatch(/top-8">Geringe Bildqualität/);
    app.limit = "renderer";
    expect(tile(false)).toMatch(/title="WebGL ist hier nicht verfügbar[^"]*"/);
    // Not on tiles that draw nothing.
    expect(
      renderToStaticMarkup(<VoiceTile stream={null} label="Alex" />),
    ).not.toContain("Geringe Bildqualität");
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
    expect(html).toContain('aria-label="Stream-Ton-Lautstärke"');
    expect(html).toContain('aria-label="Stream-Ton stummschalten"');
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
    expect(html).not.toContain("Stream-Ton");
    expect(html).not.toContain("Nicht mehr zuschauen");
  });
});
