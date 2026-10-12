import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { setNativeBridgeForTests, type NativeBridge } from "./bridge.ts";
import { NativeTrack } from "./tracks.ts";
import {
  attachNativeVideo,
  nativeVideoSuspended,
  resetNativeVideoForTests,
  type NativeVideoHost,
} from "./videoFeed.ts";
import {
  closeNativeViewer,
  nativeViewerOpen,
  openNativeViewer,
  resetNativeViewersForTests,
} from "./viewer.ts";

type Events = { onMessage: (event: Record<string, unknown>) => void };

/** The app's views and viewer windows. A command takes effect when it is
 * answered; one that is `held` is answered when the test says, in any order,
 * as the app works on commands side by side. */
class FakeApp implements NativeBridge {
  /** Commands as the page invoked them: "media_view_open 42". */
  log: string[] = [];
  held = new Set<string>();
  failing = new Set<string>();
  /** Open views with their consumer. */
  views = new Map<number, number>();
  /** Viewer windows with their event channel, by consumer. */
  windows = new Map<number, Events>();
  /** Views the app had open next to a viewer window of their consumer: it
   * sends them the window's frames, whatever they asked for. */
  beside: number[] = [];
  private pending: Array<{ command: string; answer: () => void }> = [];
  private waiting = new Map<number, (error: Error) => void>();
  private next = 0;

  async channel<T>(onMessage: (message: T) => void) {
    return { onMessage };
  }
  async invoke<T>(command: string, args: Record<string, unknown> = {}) {
    const view = args.view as number;
    const consumer = args.consumer as number;
    this.log.push(`${command} ${view ?? consumer}`);
    if (this.held.has(command))
      await new Promise<void>((answer) =>
        this.pending.push({ command, answer }),
      );
    if (this.failing.has(command)) throw new Error(`${command} failed`);
    switch (command) {
      case "media_view_open": {
        const opened = ++this.next;
        this.views.set(opened, consumer);
        if (this.windows.has(consumer)) this.beside.push(opened);
        return { view: opened } as T;
      }
      case "media_view_frame":
        if (!this.views.has(view)) throw new Error("view closed");
        // No frame: the request waits for one, or for the view's end.
        return new Promise<T>((_, reject) => this.waiting.set(view, reject));
      case "media_view_close":
        this.views.delete(view);
        this.waiting.get(view)?.(new Error("view closed"));
        this.waiting.delete(view);
        return null as T;
      case "media_viewer_open":
        this.windows.set(consumer, args.events as Events);
        for (const [open, shown] of this.views)
          if (shown === consumer) this.beside.push(open);
        return null as T;
      case "media_viewer_close":
        this.windows.delete(consumer);
        return null as T;
      default:
        return null as T;
    }
  }
  /** Answers the oldest held `command`. */
  answer(command: string): void {
    const index = this.pending.findIndex((held) => held.command === command);
    if (index < 0) throw new Error(`no ${command} waits for its answer`);
    this.pending.splice(index, 1)[0].answer();
  }
  /** What the page invoked since the last call. */
  take(): string[] {
    return this.log.splice(0);
  }
}

/** A page that lays every canvas out at 400x225 and never paints. */
const host: NativeVideoHost = {
  painter: () => ({
    painter: { load: () => true, show: () => undefined, dispose() {} },
    software: false,
  }),
  requestFrame: () => 0,
  cancelFrame: () => undefined,
  hidden: () => false,
  watch: () => () => undefined,
  box: () => ({ width: 400, height: 225 }),
  covers: () => false,
  observe: () => () => undefined,
  pixelRatio: () => 1,
};

const canvas = () => ({ width: 0, height: 0 }) as HTMLCanvasElement;
const settled = () => new Promise((resolve) => setTimeout(resolve, 0));

let app: FakeApp;
let track: NativeTrack;

beforeEach(() => {
  app = new FakeApp();
  setNativeBridgeForTests(app);
  resetNativeViewersForTests();
  resetNativeVideoForTests(host, true);
  track = new NativeTrack("video", "Bildschirm", { consumer: 42 });
});
afterEach(() => {
  resetNativeViewersForTests();
  resetNativeVideoForTests();
  setNativeBridgeForTests(undefined);
});

/** A tile that draws the stream, waiting for its next frame. */
async function tile(): Promise<() => void> {
  const leave = attachNativeVideo(track, canvas());
  await settled();
  return leave;
}

describe("viewer window of a stream the page draws", () => {
  it("opens once the app has closed the page's view of the stream", async () => {
    await tile();
    expect(app.take()).toEqual(["media_view_open 42", "media_view_frame 1"]);
    app.held.add("media_view_close");

    const opened = openNativeViewer(track, "Alex – Gelabber");
    // The tile gives way at once.
    expect(nativeViewerOpen(42)).toBe(true);
    expect(nativeVideoSuspended(42)).toBe(true);
    await settled();
    // The app still has the view, with a request waiting for the next
    // frame: a window now would answer it with its own.
    expect(app.take()).toEqual(["media_view_close 1"]);
    expect([...app.views.keys()]).toEqual([1]);

    app.answer("media_view_close");
    await opened;
    expect(app.take()).toEqual(["media_viewer_open 42"]);
    expect(app.beside).toEqual([]);
  });

  it("waits for a view the app was still opening", async () => {
    app.held.add("media_view_open");
    attachNativeVideo(track, canvas());
    await settled();
    const opened = openNativeViewer(track, "Alex – Gelabber");
    await settled();
    expect(app.take()).toEqual(["media_view_open 42"]);

    // The answer makes the view; the page closes it before the window.
    app.answer("media_view_open");
    await opened;
    expect(app.take()).toEqual(["media_view_close 1", "media_viewer_open 42"]);
    expect(app.beside).toEqual([]);
    expect(app.views.size).toBe(0);
  });

  it("opens no view while the window is open or closing", async () => {
    await tile();
    await openNativeViewer(track, "Alex – Gelabber");
    app.take();
    // A large view, a tile that mounts again.
    attachNativeVideo(track, canvas());
    await settled();
    expect(app.take()).toEqual([]);

    app.held.add("media_viewer_close");
    closeNativeViewer(42);
    expect(nativeViewerOpen(42)).toBe(false);
    await settled();
    expect(app.take()).toEqual(["media_viewer_close 42"]);
    // The app still feeds the window.
    expect(nativeVideoSuspended(42)).toBe(true);
    attachNativeVideo(track, canvas());
    await settled();
    expect(app.take()).toEqual([]);

    app.answer("media_viewer_close");
    await settled();
    expect(nativeVideoSuspended(42)).toBe(false);
    await tile();
    expect(app.take()).toEqual(["media_view_open 42", "media_view_frame 2"]);
    expect(app.beside).toEqual([]);
  });

  it("gives the stream back when the person closes the window", async () => {
    await tile();
    await openNativeViewer(track, "Alex – Gelabber");
    app.take();
    app.windows.get(42)!.onMessage({ type: "closed" });
    expect(nativeViewerOpen(42)).toBe(false);
    await settled();
    expect(app.take()).toEqual(["media_viewer_close 42"]);
    expect(nativeVideoSuspended(42)).toBe(false);
  });

  it("closes a window only after the app has opened it", async () => {
    app.held.add("media_viewer_open");
    const opened = openNativeViewer(track, "Alex – Gelabber");
    await settled();
    // "Fenster schließen" while the window is on its way.
    closeNativeViewer(42);
    await settled();
    // A close that overtook the open would leave the window behind, and
    // the tile drawing next to it.
    expect(app.take()).toEqual(["media_viewer_open 42"]);
    expect(nativeVideoSuspended(42)).toBe(true);

    app.answer("media_viewer_open");
    await opened;
    await settled();
    expect(app.take()).toEqual(["media_viewer_close 42"]);
    expect(app.windows.size).toBe(0);
    expect(nativeVideoSuspended(42)).toBe(false);
  });

  it("opens no window that was closed while the page's view closed", async () => {
    await tile();
    app.take();
    app.held.add("media_view_close");
    const opened = openNativeViewer(track, "Alex – Gelabber");
    closeNativeViewer(42);
    await settled();
    expect(app.take()).toEqual(["media_view_close 1"]);
    app.answer("media_view_close");
    await opened;
    await settled();
    expect(app.take()).toEqual(["media_viewer_close 42"]);
    expect(nativeVideoSuspended(42)).toBe(false);
  });

  it("opens a window again after the app has closed the one before", async () => {
    await openNativeViewer(track, "Alex – Gelabber");
    const first = app.windows.get(42)!;
    app.held.add("media_viewer_close");
    closeNativeViewer(42);
    const opened = openNativeViewer(track, "Alex – Gelabber");
    await settled();
    expect(app.take()).toEqual([
      "media_viewer_open 42",
      "media_viewer_close 42",
    ]);
    app.answer("media_viewer_close");
    await opened;
    expect(app.take()).toEqual(["media_viewer_open 42"]);
    // Suspended from the first window to the end of the second.
    expect(nativeVideoSuspended(42)).toBe(true);
    // The first window going away is not news about the second.
    first.onMessage({ type: "closed" });
    expect(nativeViewerOpen(42)).toBe(true);

    app.held.clear();
    closeNativeViewer(42);
    await settled();
    expect(nativeVideoSuspended(42)).toBe(false);
  });

  it("gives the stream back when the window cannot open", async () => {
    await tile();
    app.failing.add("media_viewer_open");
    await expect(openNativeViewer(track, "Alex – Gelabber")).rejects.toThrow(
      "media_viewer_open failed",
    );
    expect(nativeViewerOpen(42)).toBe(false);
    await settled();
    expect(app.take().slice(2)).toEqual([
      "media_view_close 1",
      "media_viewer_open 42",
      "media_viewer_close 42",
    ]);
    expect(nativeVideoSuspended(42)).toBe(false);
  });

  it("leaves other streams and the own camera in the page", async () => {
    const other = new NativeTrack("video", "Kamera", { consumer: 43 });
    const own = new NativeTrack("video", "Kamera", { source: 42 });
    attachNativeVideo(other, canvas());
    attachNativeVideo(own, canvas());
    await tile();
    app.take();
    await openNativeViewer(track, "Alex – Gelabber");
    expect(app.take()).toEqual(["media_view_close 3", "media_viewer_open 42"]);
    expect([...app.views.keys()]).toEqual([1, 2]);
    expect(nativeVideoSuspended(43)).toBe(false);
  });
});

describe("viewer window in an app that draws nothing in the page", () => {
  it("asks an app up to 0.5.x for its window and nothing else", async () => {
    await openNativeViewer(track, "Alex – Gelabber");
    closeNativeViewer(42);
    await settled();
    expect(app.take()).toEqual([
      "media_viewer_open 42",
      "media_viewer_close 42",
    ]);
  });
});
