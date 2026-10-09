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

import { VoiceTile } from "./VoiceTile.tsx";

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
    expect(html).toContain("Vergrößern");
    expect(html).toContain("Vollbild");
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
      '<button type="button" class="rounded-md bg-white px-3 py-2 font-medium text-neutral-900">Im Fenster ansehen</button>',
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
      '<span>Alex</span><button type="button" class="rounded-md bg-white px-3 py-2 font-medium text-neutral-900">Fenster schließen</button></div>',
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
    expect(html).toContain("Vergrößern");
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
        onToggleExpand={() => undefined}
        sourceWatch={{ watching: true, toggle: () => undefined }}
      />,
    );
    expect(surface(html)).toEqual({
      tag: "canvas",
      classes: ["size-full", "object-contain", "opacity-0"],
    });
    expect(html).not.toContain("Im Fenster ansehen");
    // The browser's controls, and the window as one more of them.
    const actions = /<div class="voice-video-actions">(.*?)<\/div>/.exec(
      html,
    )![1];
    expect(
      [...actions.matchAll(/<button[^>]*>(?:<svg.*?<\/svg>)?\s*([^<]*)/g)].map(
        (match) => match[1],
      ),
    ).toEqual([
      "Nicht mehr zuschauen",
      "",
      "Eigenes Fenster",
      "Vergrößern",
      "Vollbild",
    ]);
    expect(actions).toContain('aria-label="Im Raum hervorheben"');
    expect(actions).toContain('aria-pressed="false">Eigenes Fenster');
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
      '<span>Alex — Live</span><span role="status" class="text-xs">Läuft im eigenen Fenster.</span></div>',
    );
    // The switch that opened the window stays where it was.
    expect(html).toContain('aria-pressed="true">Fenster schließen');
    expect(html).not.toContain("Eigenes Fenster");
    expect(html).not.toContain("Im Fenster ansehen");
    expect(html).not.toContain("Geringe Bildqualität");
    expect(html).toContain("Vergrößern");
    // Another stream, and the own source with the consumer's number.
    expect(tile(stream({ consumer: 43 }), "Kim")).toContain("<canvas");
    const own = tile(stream({ source: 42 }), "Rafi — Live");
    expect(own).toContain("<canvas");
    expect(own).not.toContain("Fenster");

    // Closed: the tile waits for the app to let go of the window.
    closeNativeViewer(42);
    const closing = tile(watched, "Alex — Live");
    expect(closing).not.toContain("<canvas");
    expect(closing).toContain('aria-pressed="false">Eigenes Fenster');
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
