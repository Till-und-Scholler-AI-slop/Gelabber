import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";

import { setNativeBridgeForTests, type NativeBridge } from "./bridge.ts";
import type { Frame } from "./frames.ts";
import { NativeStream, NativeTrack } from "./tracks.ts";
import {
  attachNativeVideo,
  nativeCanvasHeight,
  nativeDisplayTrack,
  nativeVideoLimit,
  nativeVideoTarget,
  resetNativeVideoForTests,
  subscribeNativeVideoLimit,
  type NativeVideoHost,
} from "./videoFeed.ts";

/** A packet with nothing but its header set (planes are zero). */
function packet(
  seq: number,
  width = 640,
  height = 360,
  flags = 0,
): ArrayBuffer {
  const chroma = Math.ceil(width / 2) * Math.ceil(height / 2);
  const buffer = new ArrayBuffer(32 + width * height + 2 * chroma);
  const view = new DataView(buffer);
  view.setUint32(0, 0x31524647, true);
  view.setUint16(4, 32, true);
  view.setUint8(7, flags);
  view.setUint32(8, width, true);
  view.setUint32(12, height, true);
  view.setUint32(16, seq, true);
  return buffer;
}

type Pending = {
  after: number | undefined;
  resolve: (body: unknown) => void;
  reject: (error: Error) => void;
};

/** The app's view commands: one waiting frame request per view. */
class FakeApp implements NativeBridge {
  calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  waiting = new Map<number, Pending>();
  /** Holds `media_view_open` back until called. */
  release: (() => void) | null = null;
  holdOpen = false;
  failOpen = false;
  private views = 0;

  async channel() {
    return {};
  }
  named(command: string): Array<Record<string, unknown>> {
    return this.calls.filter((c) => c.command === command).map((c) => c.args);
  }
  async invoke<T>(command: string, args: Record<string, unknown> = {}) {
    this.calls.push({ command, args });
    const view = args.view as number;
    switch (command) {
      case "media_view_open":
        if (this.holdOpen)
          await new Promise<void>((resolve) => (this.release = resolve));
        if (this.failOpen) throw new Error("unknown consumer");
        return { view: ++this.views } as T;
      case "media_view_frame":
        if (this.waiting.has(view))
          throw new Error("two frame requests in flight");
        return new Promise<T>((resolve, reject) =>
          this.waiting.set(view, {
            after: args.after as number | undefined,
            resolve: resolve as (body: unknown) => void,
            reject,
          }),
        );
      case "media_view_close":
        this.end(view);
        return null as T;
      default:
        return null as T;
    }
  }
  /** Answers the waiting request of `view`. */
  deliver(view: number, body: unknown): void {
    const pending = this.waiting.get(view);
    if (!pending) throw new Error(`view ${view} asked for no frame`);
    this.waiting.delete(view);
    pending.resolve(body);
  }
  /** The view closed in the app. */
  end(view: number): void {
    this.waiting.get(view)?.reject(new Error("view closed"));
    this.waiting.delete(view);
  }
}

type FakeCanvas = HTMLCanvasElement & { name: string };

/** A page without a DOM: frames run by hand, sizes are set by the test. */
function fakePage() {
  const queue: Array<() => void> = [];
  const boxes = new Map<HTMLCanvasElement, { width: number; height: number }>();
  const covering = new Set<HTMLCanvasElement>();
  const observers = new Map<HTMLCanvasElement, () => void>();
  const page = {
    hidden: false,
    ratio: 2,
    software: false,
    lost: false,
    watcher: null as (() => void) | null,
    restore: [] as Array<() => void>,
    loads: [] as number[],
    shows: [] as string[],
    disposed: 0,
    canvas(name: string, width = 400, height = 225, cover = false) {
      const canvas = { name, width: 0, height: 0 } as FakeCanvas;
      boxes.set(canvas, { width, height });
      if (cover) covering.add(canvas);
      return canvas;
    },
    /** A new box, and the page's observer reporting it. */
    size(canvas: HTMLCanvasElement, width: number, height: number) {
      boxes.set(canvas, { width, height });
      observers.get(canvas)?.();
    },
    /** One display refresh. */
    frame() {
      for (const run of queue.splice(0)) run();
    },
    queued: () => queue.length,
    observed: () => observers.size,
  };
  const host: NativeVideoHost = {
    painter(onRestored) {
      page.restore.push(onRestored);
      let loaded: Frame | null = null;
      return {
        software: page.software,
        painter: {
          load(frame) {
            if (page.lost) return false;
            page.loads.push(frame.seq);
            loaded = frame;
            return true;
          },
          show(canvas) {
            canvas.width = loaded!.displayWidth;
            canvas.height = loaded!.displayHeight;
            page.shows.push(`${(canvas as FakeCanvas).name}:${loaded!.seq}`);
          },
          dispose: () => void page.disposed++,
        },
      };
    },
    requestFrame: (run) => queue.push(run),
    cancelFrame: () => void queue.splice(0),
    hidden: () => page.hidden,
    watch(listener) {
      page.watcher = listener;
      return () => (page.watcher = null);
    },
    box: (canvas) => boxes.get(canvas) ?? null,
    covers: (canvas) => covering.has(canvas),
    observe(canvas, listener) {
      observers.set(canvas, listener);
      return () => observers.delete(canvas);
    },
    pixelRatio: () => page.ratio,
  };
  return { page, host };
}

/** Lets promise chains run; timers stay where they are. */
async function settled(): Promise<void> {
  for (let turn = 0; turn < 10; turn++) await Promise.resolve();
}

let app: FakeApp;
let page: ReturnType<typeof fakePage>["page"];
let warn: MockInstance<typeof console.warn>;

beforeEach(() => {
  vi.useFakeTimers();
  app = new FakeApp();
  setNativeBridgeForTests(app);
  const fake = fakePage();
  page = fake.page;
  resetNativeVideoForTests(fake.host);
  warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
});
afterEach(() => {
  resetNativeVideoForTests();
  setNativeBridgeForTests(undefined);
  vi.useRealTimers();
  vi.restoreAllMocks();
});

const remote = () => new NativeTrack("video", "Bildschirm", { consumer: 42 });

describe("tracks a tile can draw", () => {
  it("takes someone's stream and the own camera or screen", () => {
    const watched = remote();
    const own = new NativeTrack("video", "Kamera", { source: 7 });
    const sound = new NativeTrack("audio", "Quellton", { source: 8 });
    expect(nativeVideoTarget(watched)).toEqual({ consumer: 42 });
    expect(nativeVideoTarget(own)).toEqual({ source: 7 });
    expect(nativeVideoTarget(sound)).toBeNull();
    expect(nativeVideoTarget(new NativeTrack("video", "x"))).toBeNull();
    const stream = (track: NativeTrack) =>
      new NativeStream([
        track as unknown as MediaStreamTrack,
      ]) as unknown as MediaStream;
    expect(nativeDisplayTrack(stream(watched))).toBe(watched);
    expect(nativeDisplayTrack(stream(own))).toBe(own);
    expect(nativeDisplayTrack(stream(sound))).toBeNull();
    expect(nativeDisplayTrack(null)).toBeNull();
    own.stop();
    expect(nativeDisplayTrack(stream(own))).toBeNull();
  });

  it("opens nothing for a track without video", async () => {
    const sound = new NativeTrack("audio", "Quellton", { source: 8 });
    const ended = remote();
    ended.stop();
    attachNativeVideo(sound, page.canvas("a"))();
    attachNativeVideo(ended, page.canvas("b"))();
    await settled();
    expect(app.calls).toEqual([]);
  });
});

describe("native video feed", () => {
  it("opens a view at the canvas's size and pulls one frame at a time", async () => {
    const track = remote();
    const painted = vi.fn();
    attachNativeVideo(track, page.canvas("tile"), painted);
    await settled();
    // 400x225 CSS pixels at a device pixel ratio of 2.
    expect(app.named("media_view_open")).toEqual([
      { consumer: 42, maxWidth: 800, maxHeight: 450 },
    ]);
    expect(app.named("media_view_frame")).toEqual([{ view: 1 }]);
    expect(app.named("media_view_configure")).toEqual([]);

    app.deliver(1, packet(1));
    await settled();
    // The next request is out while the frame waits for the display.
    expect(app.named("media_view_frame")).toEqual([
      { view: 1 },
      { view: 1, after: 1 },
    ]);
    expect(page.shows).toEqual([]);
    expect(page.queued()).toBe(1);
    page.frame();
    expect(page.shows).toEqual(["tile:1"]);
    expect(painted).toHaveBeenCalledTimes(1);
    // Nothing new: the display has nothing to do.
    expect(page.queued()).toBe(0);

    app.deliver(1, packet(2));
    await settled();
    page.frame();
    expect(page.shows).toEqual(["tile:1", "tile:2"]);
    expect(painted).toHaveBeenCalledTimes(1);
    expect(app.named("media_view_frame")).toHaveLength(3);
  });

  it("opens a view on the capture source of an own track", async () => {
    const own = new NativeTrack("video", "Kamera", { source: 7 });
    attachNativeVideo(own, page.canvas("self"));
    await settled();
    expect(app.named("media_view_open")).toEqual([
      { source: 7, maxWidth: 800, maxHeight: 450 },
    ]);
  });

  it("skips to the newest frame when the page paints slower than the stream", async () => {
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    app.deliver(1, packet(1));
    await settled();
    app.deliver(1, packet(2));
    await settled();
    // Two frames arrived without a paint: no third request before one.
    expect(app.named("media_view_frame")).toHaveLength(2);
    expect(app.waiting.size).toBe(0);
    page.frame();
    expect(page.loads).toEqual([2]);
    expect(page.shows).toEqual(["tile:2"]);
    expect(app.named("media_view_frame").at(-1)).toEqual({ view: 1, after: 2 });
  });

  it("shares one view between the canvases of a track", async () => {
    const track = remote();
    const tile = page.canvas("tile");
    const large = page.canvas("large", 1000, 500);
    const tilePainted = vi.fn();
    const largePainted = vi.fn();
    const leaveTile = attachNativeVideo(track, tile, tilePainted);
    await settled();
    app.deliver(1, packet(1));
    await settled();
    page.frame();

    // The large view opens: it starts with the picture the tile shows.
    const leaveLarge = attachNativeVideo(track, large, largePainted);
    expect(page.shows).toEqual(["tile:1", "large:1"]);
    expect(largePainted).toHaveBeenCalledTimes(1);
    await settled();
    expect(app.named("media_view_open")).toHaveLength(1);
    // The app scales to the largest canvas, once the size has held.
    expect(app.named("media_view_configure")).toEqual([]);
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 2000, maxHeight: 1000 },
    ]);

    app.deliver(1, packet(2, 1280, 720));
    await settled();
    page.frame();
    expect(page.loads).toEqual([1, 1, 2]);
    expect(page.shows.slice(2)).toEqual(["tile:2", "large:2"]);
    expect(tilePainted).toHaveBeenCalledTimes(1);

    leaveLarge();
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure").at(-1)).toEqual({
      view: 1,
      maxWidth: 800,
      maxHeight: 450,
    });
    expect(app.named("media_view_close")).toEqual([]);

    // The last canvas leaves: the view closes, the waiting request with it.
    leaveTile();
    leaveTile();
    await vi.advanceTimersByTimeAsync(250);
    expect(app.named("media_view_close")).toEqual([{ view: 1 }]);
    await settled();
    expect(app.named("media_view_close")).toHaveLength(1);
    expect(app.named("media_view_open")).toHaveLength(1);
    expect(page.disposed).toBe(1);
    expect(page.observed()).toBe(0);
    expect(page.watcher).toBeNull();
  });

  it("keeps the view and the picture while a tile moves", async () => {
    const track = remote();
    const leave = attachNativeVideo(track, page.canvas("grid"));
    await settled();
    app.deliver(1, packet(1));
    await settled();
    page.frame();
    // Room focus: the tile unmounts and mounts elsewhere in one commit.
    leave();
    const painted = vi.fn();
    attachNativeVideo(track, page.canvas("focus"), painted);
    expect(painted).toHaveBeenCalledTimes(1);
    expect(page.shows).toEqual(["grid:1", "focus:1"]);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(app.named("media_view_open")).toHaveLength(1);
    expect(app.named("media_view_close")).toEqual([]);
  });

  it("asks for nothing while no canvas shows the stream", async () => {
    const track = remote();
    const leave = attachNativeVideo(track, page.canvas("tile"));
    await settled();
    leave();
    // The request that was out comes back; no canvas, no next one.
    app.deliver(1, packet(1));
    await settled();
    page.frame();
    expect(app.named("media_view_frame")).toHaveLength(1);
    expect(page.shows).toEqual([]);
    // A canvas within the grace period takes up where the feed stopped.
    attachNativeVideo(track, page.canvas("again"));
    expect(page.shows).toEqual(["again:1"]);
    expect(app.named("media_view_frame").at(-1)).toEqual({ view: 1, after: 1 });
  });

  it("reports a new size only when it differs enough and has held", async () => {
    const tile = page.canvas("tile");
    attachNativeVideo(remote(), tile);
    await settled();
    // 5 % larger: the picture the app sends still does.
    page.size(tile, 420, 236);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(app.named("media_view_configure")).toEqual([]);

    // A window being dragged larger: one report, after it came to rest.
    for (const width of [500, 600, 700, 800]) {
      page.size(tile, width, (width * 9) / 16);
      await vi.advanceTimersByTimeAsync(50);
    }
    expect(app.named("media_view_configure")).toEqual([]);
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 1600, maxHeight: 900 },
    ]);

    // Smaller again, and odd sizes go up to even ones.
    page.size(tile, 300.4, 169.2);
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure").at(-1)).toEqual({
      view: 1,
      maxWidth: 602,
      maxHeight: 340,
    });

    // The window moved to a display with another pixel ratio.
    page.ratio = 1;
    page.watcher?.();
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure").at(-1)).toEqual({
      view: 1,
      maxWidth: 302,
      maxHeight: 170,
    });
  });

  it("sends the first size without waiting once a canvas has one", async () => {
    const track = remote();
    // No layout yet: the view opens with the app's default size.
    const tile = { name: "tile", width: 0, height: 0 } as FakeCanvas;
    attachNativeVideo(track, tile);
    await settled();
    expect(app.named("media_view_open")).toEqual([{ consumer: 42 }]);
    expect(app.named("media_view_configure")).toEqual([]);
    const late = page.canvas("late", 320, 180);
    attachNativeVideo(track, late);
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 640, maxHeight: 360 },
    ]);
  });

  it("never asks for more than 1080p worth of pixels", async () => {
    // Fullscreen on a 3840x2160 display.
    attachNativeVideo(remote(), page.canvas("tile", 1920, 1080));
    await settled();
    expect(app.named("media_view_open")).toEqual([
      { consumer: 42, maxWidth: 1920, maxHeight: 1080 },
    ]);
  });

  it("asks for a picture that fills a cropping canvas", async () => {
    // A 4:3 camera in a 16:9 tile with object-fit: cover.
    const tile = page.canvas("tile", 400, 225, true);
    attachNativeVideo(remote(), tile);
    await settled();
    expect(app.named("media_view_open")).toEqual([
      { consumer: 42, maxWidth: 800, maxHeight: 450 },
    ]);
    // The app fits 4:3 into that: 600x450, too narrow for the tile.
    app.deliver(1, packet(1, 600, 450));
    await settled();
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 800, maxHeight: 600 },
    ]);
    // And that is stable.
    app.deliver(1, packet(2, 800, 600));
    await settled();
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure")).toHaveLength(1);
  });

  it("asks for the longer side both ways for a turned picture", async () => {
    // A phone held upright: 640x360 planes, shown as 360x640.
    attachNativeVideo(remote(), page.canvas("tile", 200, 300));
    await settled();
    app.deliver(1, packet(1, 640, 360, 0b010));
    await settled();
    await vi.advanceTimersByTimeAsync(150);
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 600, maxHeight: 600 },
    ]);
  });

  it("pauses while the document is hidden", async () => {
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    app.deliver(1, packet(1));
    await settled();
    page.frame();
    expect(app.named("media_view_frame")).toHaveLength(2);

    page.hidden = true;
    page.watcher?.();
    // The request that was out is answered; no next one.
    app.deliver(1, packet(2));
    await settled();
    expect(app.named("media_view_frame")).toHaveLength(2);
    expect(app.waiting.size).toBe(0);

    page.hidden = false;
    page.watcher?.();
    expect(app.named("media_view_frame").at(-1)).toEqual({ view: 1, after: 2 });
    expect(app.named("media_view_frame")).toHaveLength(3);
  });

  it("does not start pulling in a hidden document", async () => {
    page.hidden = true;
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    expect(app.named("media_view_open")).toHaveLength(1);
    expect(app.named("media_view_frame")).toEqual([]);
    page.hidden = false;
    page.watcher?.();
    expect(app.named("media_view_frame")).toEqual([{ view: 1 }]);
  });

  it("stops when the app closes the view", async () => {
    const track = remote();
    const tile = page.canvas("tile");
    const leave = attachNativeVideo(track, tile);
    await settled();
    app.deliver(1, packet(1));
    await settled();
    page.frame();
    // The consumer was freed: the waiting request rejects.
    app.end(1);
    await settled();
    expect(app.named("media_view_frame")).toHaveLength(2);
    expect(app.named("media_view_close")).toEqual([{ view: 1 }]);
    expect(nativeCanvasHeight(track.id)).toBe(0);
    expect(page.disposed).toBe(1);
    expect(warn).not.toHaveBeenCalled();
    // The tile lets go afterwards, as React gets to it.
    leave();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(app.named("media_view_close")).toHaveLength(1);
    expect(app.named("media_view_open")).toHaveLength(1);
    expect(page.observed()).toBe(0);
  });

  it("opens a new view for a canvas that comes after the old one ended", async () => {
    const track = remote();
    attachNativeVideo(track, page.canvas("tile"));
    await settled();
    app.end(1);
    await settled();
    attachNativeVideo(track, page.canvas("large"));
    await settled();
    expect(app.named("media_view_open")).toHaveLength(2);
    expect(app.named("media_view_frame").at(-1)).toEqual({ view: 2 });
  });

  it("closes a view that opens after its canvases left", async () => {
    app.holdOpen = true;
    const leave = attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    leave();
    await vi.advanceTimersByTimeAsync(250);
    expect(app.named("media_view_close")).toEqual([]);
    app.release?.();
    await settled();
    expect(app.named("media_view_close")).toEqual([{ view: 1 }]);
    expect(app.named("media_view_frame")).toEqual([]);
  });

  it("gives up quietly when the track ended before its view opened", async () => {
    app.holdOpen = true;
    app.failOpen = true;
    const track = remote();
    const leave = attachNativeVideo(track, page.canvas("tile"));
    await settled();
    track.stop();
    app.release?.();
    await settled();
    expect(warn).not.toHaveBeenCalled();
    expect(app.named("media_view_frame")).toEqual([]);
    leave();
    await vi.advanceTimersByTimeAsync(1_000);
    expect(app.named("media_view_close")).toEqual([]);
  });

  it("says why a live track got no view", async () => {
    app.failOpen = true;
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(app.named("media_view_frame")).toEqual([]);
  });

  it("stops on a packet it cannot read", async () => {
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    app.deliver(1, new ArrayBuffer(64));
    await settled();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(app.named("media_view_close")).toEqual([{ view: 1 }]);
    expect(app.named("media_view_frame")).toHaveLength(1);
  });

  it("paints again when a lost WebGL context is back", async () => {
    const painted = vi.fn();
    attachNativeVideo(remote(), page.canvas("tile"), painted);
    await settled();
    page.lost = true;
    app.deliver(1, packet(1));
    await settled();
    page.frame();
    expect(painted).not.toHaveBeenCalled();
    expect(page.queued()).toBe(0);
    page.lost = false;
    page.restore[0]();
    page.frame();
    expect(page.shows).toEqual(["tile:1"]);
    expect(painted).toHaveBeenCalledTimes(1);
  });

  it("measures the picture's height for the layer choice", async () => {
    const track = remote();
    // A 4:3 box.
    const tile = page.canvas("tile", 480, 360);
    attachNativeVideo(track, tile);
    await settled();
    // Before the first picture the box counts.
    expect(nativeCanvasHeight(track.id)).toBe(720);
    app.deliver(1, packet(1, 1280, 720));
    await settled();
    page.frame();
    // 16:9 letterboxed in it: 480 * 9 / 16 lines, times the pixel ratio.
    expect(nativeCanvasHeight(track.id)).toBe(540);
    // The largest canvas decides; one without layout does not count.
    const large = page.canvas("large", 1600, 900);
    attachNativeVideo(track, large);
    expect(nativeCanvasHeight(track.id)).toBe(1800);
    attachNativeVideo(track, { name: "x", width: 0, height: 0 } as FakeCanvas);
    expect(nativeCanvasHeight(track.id)).toBe(1800);
    expect(nativeCanvasHeight("other")).toBe(0);
  });
});

describe("in-page video that has to stay small", () => {
  it("detects frames that arrive as JSON and stops asking for large ones", async () => {
    const changed = vi.fn();
    subscribeNativeVideoLimit(changed);
    const track = remote();
    const painted = vi.fn();
    attachNativeVideo(track, page.canvas("tile", 800, 450), painted);
    await settled();
    expect(app.named("media_view_open")).toEqual([
      { consumer: 42, maxWidth: 1600, maxHeight: 900 },
    ]);
    expect(nativeVideoLimit()).toBeNull();

    // Tauri's postMessage fallback: the bytes as a number array.
    app.deliver(1, Array.from(new Uint8Array(packet(1, 1280, 720))));
    await settled();
    expect(nativeVideoLimit()).toBe("transport");
    expect(changed).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0][0])).toContain("connect-src");
    // At once, not after the usual wait.
    expect(app.named("media_view_configure")).toEqual([
      { view: 1, maxWidth: 320, maxHeight: 180, maxFps: 15 },
    ]);
    // The frame that did arrive is shown.
    page.frame();
    expect(page.shows).toEqual(["tile:1"]);
    expect(painted).toHaveBeenCalledTimes(1);
    // The server is asked for the low layer.
    expect(nativeCanvasHeight(track.id)).toBe(180);

    app.deliver(1, Array.from(new Uint8Array(packet(2, 320, 180))));
    await settled();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(changed).toHaveBeenCalledTimes(1);

    // Later views are small from the start, whatever their canvas.
    attachNativeVideo(
      new NativeTrack("video", "Kamera", { source: 7 }),
      page.canvas("self", 100, 50),
    );
    attachNativeVideo(new NativeTrack("video", "x", { consumer: 43 }), {
      name: "no layout",
      width: 0,
      height: 0,
    } as FakeCanvas);
    await settled();
    expect(app.named("media_view_open").slice(1)).toEqual([
      { source: 7, maxWidth: 200, maxHeight: 100, maxFps: 15 },
      { consumer: 43, maxWidth: 320, maxHeight: 180, maxFps: 15 },
    ]);
  });

  it("takes a typed array for what it is", async () => {
    attachNativeVideo(remote(), page.canvas("tile"));
    await settled();
    const bytes = new Uint8Array(8 + packet(1).byteLength);
    bytes.set(new Uint8Array(packet(1)), 8);
    app.deliver(1, bytes.subarray(8));
    await settled();
    page.frame();
    expect(page.shows).toEqual(["tile:1"]);
    expect(nativeVideoLimit()).toBeNull();
  });

  it("keeps video small on a page without WebGL", async () => {
    page.software = true;
    attachNativeVideo(remote(), page.canvas("tile", 800, 450));
    await settled();
    expect(nativeVideoLimit()).toBe("renderer");
    expect(warn).toHaveBeenCalledTimes(1);
    expect(app.named("media_view_open")).toEqual([
      { consumer: 42, maxWidth: 320, maxHeight: 180, maxFps: 15 },
    ]);
  });
});
