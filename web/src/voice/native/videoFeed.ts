// Native video in the page: one feed per track, shared by every canvas that
// shows it (tile, large view). The feed opens a view on the track's consumer
// or capture source in the app, pulls its frames with one request at a time
// and paints the newest one per display refresh. The app scales frames to
// the size the feed reports, so a small tile costs little.
//
// Not next to a viewer window (viewer.ts): the app feeds the window and the
// views of a consumer from one sink and hands the views the window's frames,
// unscaled (desktop/README.md), a 4K picture where a tile asked for 320x180.
// So the page has no view of a consumer while a window shows it
// (`suspendNativeVideo`).
import { invokeNative } from "./bridge.ts";
import {
  createFramePainter,
  parsePacket,
  type Frame,
  type FramePainter,
} from "./frames.ts";
import { isNativeTrack, type NativeTrack } from "./tracks.ts";

/** What a view of a native video track attaches to in the app. */
export type NativeVideoTarget = { consumer: number } | { source: number };

export function nativeVideoTarget(
  track: NativeTrack,
): NativeVideoTarget | null {
  if (track.kind !== "video") return null;
  if (track.handle.consumer !== undefined)
    return { consumer: track.handle.consumer };
  if (track.handle.source !== undefined) return { source: track.handle.source };
  return null;
}

/** The live video track a tile can draw: someone's stream (a consumer) or
 * the own camera, screen or Live (a capture source). */
export function nativeDisplayTrack(
  stream: MediaStream | null,
): NativeTrack | null {
  const track = stream?.getVideoTracks()[0];
  return isNativeTrack(track) &&
    track.readyState === "live" &&
    nativeVideoTarget(track) !== null
    ? track
    : null;
}

/** A view the last canvas left stays open this long: a tile that moves
 * (room focus, remount) attaches again right away and keeps its picture. */
const LINGER_MS = 250;
/** A new size has to hold this long (window resizes, animations) ... */
const SETTLE_MS = 150;
/** ... and differ this much from what the app delivers, to be reported. */
const HYSTERESIS = 0.1;
/** Most pixels a view asks for, give or take a row. Carrying frames into
 * the page costs per byte: one 1080p30 stream took 0.4 of a core on each
 * side when measured, four times that at 2160p. Larger canvases scale the
 * picture up. */
const MAX_PIXELS = 1920 * 1080;
/** What a view gets when frames cannot be carried or converted quickly, and
 * the size of every view until the page knows that they can. */
const LIMITED = { maxWidth: 320, maxHeight: 180, maxFps: 15 };

/** `transport`: the app's binary IPC is blocked for this origin and frames
 * arrive as JSON number arrays. `renderer`: the page has no WebGL. */
export type NativeVideoLimit = "transport" | "renderer";

type Request = { maxWidth: number; maxHeight: number; maxFps?: number };
type Box = { width: number; height: number };

/** The page around the feeds; tests bring their own. */
export type NativeVideoHost = {
  painter(onRestored: () => void): { painter: FramePainter; software: boolean };
  requestFrame(run: () => void): number;
  cancelFrame(id: number): void;
  hidden(): boolean;
  /** Runs `listener` when visibility or the pixel ratio changed. */
  watch(listener: () => void): () => void;
  /** A canvas's box in CSS pixels; null while it has no layout. */
  box(canvas: HTMLCanvasElement): Box | null;
  /** Whether the picture fills the box and is cropped (`object-fit: cover`)
   * instead of fitting inside it. */
  covers(canvas: HTMLCanvasElement): boolean;
  /** Runs `listener` when the canvas's box changed. */
  observe(canvas: HTMLCanvasElement, listener: () => void): () => void;
  pixelRatio(): number;
};

const page: NativeVideoHost = {
  painter: createFramePainter,
  requestFrame: (run) => requestAnimationFrame(run),
  cancelFrame: (id) => cancelAnimationFrame(id),
  hidden: () => document.visibilityState === "hidden",
  watch(listener) {
    document.addEventListener("visibilitychange", listener);
    // A resolution query matches one ratio: ask again after each change.
    let query: MediaQueryList | null = null;
    const ratioChanged = () => {
      arm();
      listener();
    };
    const arm = () => {
      if (typeof matchMedia !== "function") return;
      query = matchMedia(
        `(resolution: ${globalThis.devicePixelRatio || 1}dppx)`,
      );
      query.addEventListener("change", ratioChanged, { once: true });
    };
    arm();
    return () => {
      document.removeEventListener("visibilitychange", listener);
      query?.removeEventListener("change", ratioChanged);
    };
  },
  box(canvas) {
    const rect = canvas.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0
      ? { width: rect.width, height: rect.height }
      : null;
  },
  covers: (canvas) => getComputedStyle(canvas).objectFit === "cover",
  observe(canvas, listener) {
    if (typeof ResizeObserver === "undefined") return () => undefined;
    const observer = new ResizeObserver(listener);
    observer.observe(canvas);
    return () => observer.disconnect();
  },
  pixelRatio: () => globalThis.devicePixelRatio || 1,
};

let host = page;
/** Live feeds by track id. */
const feeds = new Map<string, Feed>();
let unwatch: (() => void) | null = null;
let raf: number | null = null;
let limit: NativeVideoLimit | null = null;
const limitListeners = new Set<() => void>();
/** A frame has arrived as bytes on this page. Only the first answer tells
 * how the app's IPC carries frames here, and a large one as JSON would be
 * millions of numbers: until then every view asks for a small picture. */
let carried = false;
/** Consumers the page shows no video of. `count`: how many times that was
 * asked for (a viewer window that is closing and its successor overlap).
 * `closed` settles when the app has closed the views there were. */
const suspended = new Map<number, { count: number; closed: Promise<void> }>();
const suspensionListeners = new Set<() => void>();
/** The feeds a refresh paints, and the picture size the last one left the
 * painters' canvas at. */
const due: Feed[] = [];
let left = 0;

/** Same sizes in a row, starting with the one the canvas already has. */
function bySize(a: Feed, b: Feed): number {
  const first = a.pending();
  const second = b.pending();
  return (first === left ? 0 : first) - (second === left ? 0 : second);
}

function paintAll(): void {
  raf = null;
  for (const feed of feeds.values()) if (feed.pending() > 0) due.push(feed);
  // The painters' canvas may be reallocated for each new size (frames.ts).
  if (due.length > 1) due.sort(bySize);
  try {
    for (let index = 0; index < due.length; index++) {
      left = due[index].pending();
      due[index].paint();
    }
  } finally {
    due.length = 0;
  }
}

function schedulePaint(): void {
  raf ??= host.requestFrame(paintAll);
}

function pageChanged(): void {
  for (const feed of feeds.values()) feed.pageChanged();
}

function limitTo(reason: NativeVideoLimit): void {
  if (limit !== null) return;
  limit = reason;
  console.warn(
    reason === "transport"
      ? `[gelabber] in-app video: Tauri's IPC fetch is blocked for this origin, frames arrive as JSON. Video stays at ${LIMITED.maxWidth}x${LIMITED.maxHeight}, ${LIMITED.maxFps} fps. A Content-Security-Policy on the server has to allow "ipc:" and "http://ipc.localhost" in connect-src.`
      : `[gelabber] in-app video: no WebGL in this page, frames are converted in script. Video stays at ${LIMITED.maxWidth}x${LIMITED.maxHeight}, ${LIMITED.maxFps} fps.`,
  );
  for (const feed of feeds.values()) feed.resize(true);
  for (const listener of [...limitListeners]) listener();
}

/** The app's IPC carries frames as bytes: the views may grow to what their
 * canvases show. */
function bytesCarried(): void {
  if (carried) return;
  carried = true;
  for (const feed of feeds.values()) feed.resize(true);
}

/** Why in-page video is kept small on this page, if it is. */
export function nativeVideoLimit(): NativeVideoLimit | null {
  return limit;
}

export function subscribeNativeVideoLimit(listener: () => void): () => void {
  limitListeners.add(listener);
  return () => limitListeners.delete(listener);
}

/** The packet of a `media_view_frame` answer. Tauri hands a raw response
 * over as an ArrayBuffer; when its IPC fetch is blocked it falls back to
 * postMessage and the bytes arrive as a JSON number array. */
function packet(body: unknown): ArrayBuffer {
  if (body instanceof ArrayBuffer) return body;
  if (ArrayBuffer.isView(body))
    return body.buffer.slice(
      body.byteOffset,
      body.byteOffset + body.byteLength,
    ) as ArrayBuffer;
  if (!Array.isArray(body)) throw new Error("not a frame packet");
  limitTo("transport");
  return Uint8Array.from(body as number[]).buffer;
}

/** Up to the next even number; arithmetic's last digits do not count. */
const even = (pixels: number) => 2 * Math.ceil(pixels / 2 - 1e-6);

/** A command whose failure changes nothing here: the view is gone anyway.
 * Resolves when the app has answered, whatever it said. */
function tell(command: string, args: Record<string, unknown>): Promise<void> {
  try {
    return invokeNative(command, args).then(
      () => undefined,
      () => undefined,
    );
  } catch {
    // No app around the page any more.
    return Promise.resolve();
  }
}

type Canvas = {
  canvas: HTMLCanvasElement;
  onFrame: () => void;
  unobserve: () => void;
  /** Shows a frame of this feed. */
  painted: boolean;
};

class Feed {
  private canvases: Canvas[] = [];
  private readonly painter: FramePainter;
  /** `open` from the app's answer on; `closed` is final. */
  private state: "new" | "opening" | "open" | "closed" = "new";
  private view: number | null = null;
  /** The app's answer to `media_view_open`: the view, or null without one. */
  private opened: Promise<number | null> | null = null;
  /** From `close` on: settles when the app has no view of this feed left. */
  private ended: Promise<void> | null = null;
  private inFlight = false;
  private latest: Frame | null = null;
  /** `latest` is not on the canvases yet. */
  private unpainted = false;
  /** A frame was replaced before it was painted: the page is slower than
   * the stream, so the next request waits for a paint. */
  private behind = false;
  /** The size the app has for the view. */
  private reported: Request | null = null;
  private linger: ReturnType<typeof setTimeout> | undefined;
  private settle: ReturnType<typeof setTimeout> | undefined;

  constructor(
    private readonly track: NativeTrack,
    private readonly target: NativeVideoTarget,
  ) {
    const { painter, software } = host.painter(() => this.restored());
    this.painter = painter;
    if (software) limitTo("renderer");
  }

  get live(): boolean {
    return this.state !== "closed";
  }

  /** Whether this is the feed of that consumer's stream. */
  shows(consumer: number): boolean {
    return "consumer" in this.target && this.target.consumer === consumer;
  }

  attach(canvas: HTMLCanvasElement, onFrame: () => void): void {
    clearTimeout(this.linger);
    const entry: Canvas = {
      canvas,
      onFrame,
      unobserve: host.observe(canvas, () => this.resize()),
      painted: false,
    };
    this.canvases.push(entry);
    if (this.state === "new") this.opened = this.open();
    // A canvas that joins a running feed starts with its current picture.
    if (this.latest && this.painter.load(this.latest)) this.show(entry);
    this.resize();
    this.pull();
  }

  detach(canvas: HTMLCanvasElement): void {
    const index = this.canvases.findIndex((entry) => entry.canvas === canvas);
    if (index < 0) return;
    this.canvases.splice(index, 1)[0].unobserve();
    if (this.state === "closed") return;
    if (this.canvases.length > 0) return this.resize();
    clearTimeout(this.linger);
    this.linger = setTimeout(() => void this.close(), LINGER_MS);
  }

  /** Resolves with the view the app opened, or with null when it gave none. */
  private async open(): Promise<number | null> {
    this.state = "opening";
    const request = this.request();
    let view: number;
    try {
      ({ view } = await invokeNative<{ view: number }>("media_view_open", {
        ...this.target,
        ...request,
      }));
    } catch (error) {
      // A track that ended in the meantime has nothing to show.
      if (this.live && this.track.readyState === "live")
        console.warn("[gelabber] in-app video: no view", error);
      void this.close();
      return null;
    }
    // The feed ended while the app was opening the view: `close` waits for
    // this answer and closes it.
    if (!this.live) return view;
    this.state = "open";
    this.view = view;
    this.reported = request;
    this.configure();
    this.pull();
    return view;
  }

  /** Asks for the next frame, unless a request is out or nothing would
   * show it. */
  private pull(): void {
    if (
      this.state !== "open" ||
      this.inFlight ||
      this.behind ||
      this.canvases.length === 0 ||
      host.hidden()
    )
      return;
    this.inFlight = true;
    const after = this.latest?.seq;
    invokeNative<unknown>(
      "media_view_frame",
      after === undefined ? { view: this.view } : { view: this.view, after },
    ).then(
      (body) => {
        this.inFlight = false;
        this.arrived(body);
      },
      () => {
        // The view is gone: its consumer or source was freed.
        this.inFlight = false;
        void this.close();
      },
    );
  }

  private arrived(body: unknown): void {
    if (!this.live) return;
    let frame: Frame;
    try {
      frame = parsePacket(packet(body));
    } catch (error) {
      console.warn("[gelabber] in-app video: bad frame", error);
      void this.close();
      return;
    }
    const previous = this.latest;
    this.latest = frame;
    if (this.unpainted) this.behind = true;
    this.unpainted = true;
    // The picture's shape decides what the canvases need; the first one
    // has no reason to wait.
    if (
      previous?.displayWidth !== frame.displayWidth ||
      previous.displayHeight !== frame.displayHeight
    )
      this.resize(previous === null);
    // After the frame, so that the size asked for fits its shape.
    if (limit === null) bytesCarried();
    schedulePaint();
    this.pull();
  }

  /** Picture size of the frame `paint` would draw, as one number; 0 when
   * there is nothing new. */
  pending(): number {
    const frame = this.latest;
    return frame && this.unpainted
      ? frame.displayWidth * 0x10000 + frame.displayHeight
      : 0;
  }

  /** Per display refresh: the newest frame onto the canvases. */
  paint(): void {
    const frame = this.latest;
    if (!frame || !this.unpainted) return;
    this.unpainted = false;
    if (this.canvases.length > 0 && this.painter.load(frame))
      for (let index = 0; index < this.canvases.length; index++)
        this.show(this.canvases[index]);
    if (this.behind) {
      this.behind = false;
      this.pull();
    }
  }

  private show(entry: Canvas): void {
    this.painter.show(entry.canvas);
    if (entry.painted) return;
    entry.painted = true;
    entry.onFrame();
  }

  /** The WebGL context is back after a loss: paint again. */
  private restored(): void {
    if (!this.latest) return;
    this.unpainted = true;
    schedulePaint();
  }

  pageChanged(): void {
    this.resize();
    this.pull();
  }

  /** The largest picture the canvases can use, in physical pixels; null
   * while no canvas has a layout. */
  private wanted(): Request | null {
    const ratio = host.pixelRatio();
    const frame = this.latest;
    let width = 0;
    let height = 0;
    for (const { canvas } of this.canvases) {
      const box = host.box(canvas);
      if (!box) continue;
      let wide = box.width * ratio;
      let high = box.height * ratio;
      // Once the picture's shape is known, the picture counts instead of
      // the box: it fills a cropping canvas and fits inside any other.
      if (frame) {
        const byWidth = wide / frame.displayWidth;
        const byHeight = high / frame.displayHeight;
        if (host.covers(canvas) ? byWidth > byHeight : byWidth < byHeight)
          high = frame.displayHeight * byWidth;
        else wide = frame.displayWidth * byHeight;
      }
      width = Math.max(width, wide);
      height = Math.max(height, high);
    }
    if (width === 0) return null;
    // A turned picture: whichever way round the app reads the limits, the
    // longer side has to fit.
    if (frame && frame.displayWidth !== frame.width)
      width = height = Math.max(width, height);
    // Over the limit: the same shape with fewer pixels. Rounded up like any
    // other size: the frames the shape is taken from are rounded too, and a
    // 1080p picture has to fit either way.
    const shrink = Math.min(1, Math.sqrt(MAX_PIXELS / (width * height)));
    return {
      maxWidth: even(width * shrink),
      maxHeight: even(height * shrink),
    };
  }

  /** What the app is asked for: `wanted`, or no more than a small picture
   * while the page cannot take large ones or does not know yet. */
  private request(): Request | null {
    const wanted = this.wanted();
    if (limit === null && carried) return wanted;
    const small: Request = {
      maxWidth: Math.min(
        wanted?.maxWidth ?? LIMITED.maxWidth,
        LIMITED.maxWidth,
      ),
      maxHeight: Math.min(
        wanted?.maxHeight ?? LIMITED.maxHeight,
        LIMITED.maxHeight,
      ),
    };
    if (limit !== null) small.maxFps = LIMITED.maxFps;
    return small;
  }

  /** A canvas came, went or changed its box. `now` skips the wait. */
  resize(now = false): void {
    if (!this.live) return;
    clearTimeout(this.settle);
    // The first size goes out at once: until then the app sends 1280x720.
    if (now || this.reported === null) this.configure();
    else this.settle = setTimeout(() => this.configure(), SETTLE_MS);
  }

  private configure(): void {
    if (this.state !== "open") return;
    const request = this.request();
    const reported = this.reported;
    if (
      !request ||
      (reported &&
        request.maxFps === reported.maxFps &&
        Math.abs(request.maxWidth - reported.maxWidth) <=
          reported.maxWidth * HYSTERESIS &&
        Math.abs(request.maxHeight - reported.maxHeight) <=
          reported.maxHeight * HYSTERESIS)
    )
      return;
    this.reported = request;
    void tell("media_view_configure", { view: this.view, ...request });
  }

  /** Height of the picture the page shows, in physical pixels: the measure
   * `renderedVideoHeight` takes of a <video>. */
  shownHeight(): number {
    let height = 0;
    for (const { canvas } of this.canvases) {
      const box = host.box(canvas);
      if (!box) continue;
      // The canvas's bitmap is the picture; before the first one the box
      // counts.
      const picture =
        canvas.width > 0 && canvas.height > 0
          ? Math.min(box.height, (box.width * canvas.height) / canvas.width)
          : box.height;
      height = Math.max(height, picture * host.pixelRatio());
    }
    // A limited view shows no more than this, whatever the canvas.
    return limit === null ? height : Math.min(height, LIMITED.maxHeight);
  }

  /** Ends the feed. Resolves once the app has closed its view, also one it
   * was still opening: the app makes no frame for this feed after that. */
  close(): Promise<void> {
    if (!this.live) return this.ended ?? Promise.resolve();
    this.state = "closed";
    clearTimeout(this.linger);
    clearTimeout(this.settle);
    if (feeds.get(this.track.id) === this) feeds.delete(this.track.id);
    if (feeds.size === 0) {
      unwatch?.();
      unwatch = null;
      if (raf !== null) host.cancelFrame(raf);
      raf = null;
    }
    this.painter.dispose();
    // The canvases keep their last picture until the tile lets go of them.
    this.latest = null;
    const view = this.view;
    this.view = null;
    this.ended =
      view !== null
        ? tell("media_view_close", { view })
        : (this.opened ?? Promise.resolve(null)).then((late) =>
            late === null
              ? undefined
              : tell("media_view_close", { view: late }),
          );
    return this.ended;
  }
}

/** Draws `track` on `canvas` until the returned function is called. Several
 * canvases may show one track; the app delivers its frames once. `onFrame`
 * runs when the canvas shows its first picture. Draws nothing for a consumer
 * that is suspended: its tiles show something else until it is resumed and
 * attach a new canvas then (`subscribeNativeVideoSuspensions`). */
export function attachNativeVideo(
  track: NativeTrack,
  canvas: HTMLCanvasElement,
  onFrame: () => void = () => undefined,
): () => void {
  const target = nativeVideoTarget(track);
  if (!target || track.readyState !== "live") return () => undefined;
  if ("consumer" in target && suspended.has(target.consumer))
    return () => undefined;
  let feed = feeds.get(track.id);
  if (!feed) {
    feed = new Feed(track, target);
    feeds.set(track.id, feed);
    unwatch ??= host.watch(pageChanged);
  }
  const attached = feed;
  attached.attach(canvas, onFrame);
  return () => attached.detach(canvas);
}

/** Height the page shows a native track at, in physical pixels; 0 when no
 * canvas shows it. */
export function nativeCanvasHeight(trackId: string): number {
  return feeds.get(trackId)?.shownHeight() ?? 0;
}

function suspensionsChanged(): void {
  for (const listener of [...suspensionListeners]) listener();
}

/** Takes `consumer`'s video out of the page until `resumeNativeVideo`: its
 * views close now and none opens meanwhile. Resolves once the app has closed
 * them, also one it was still opening: whatever the app does with the
 * consumer after that reaches no view of the page. */
export function suspendNativeVideo(consumer: number): Promise<void> {
  const already = suspended.get(consumer);
  if (already) {
    already.count++;
    return already.closed;
  }
  const suspension = { count: 1, closed: Promise.resolve() };
  suspended.set(consumer, suspension);
  const closing: Promise<void>[] = [];
  for (const feed of [...feeds.values()])
    if (feed.shows(consumer)) closing.push(feed.close());
  suspension.closed = Promise.all(closing).then(() => undefined);
  suspensionsChanged();
  return suspension.closed;
}

/** Ends one `suspendNativeVideo` of `consumer`; after the last one its
 * tiles may draw it again. */
export function resumeNativeVideo(consumer: number): void {
  const suspension = suspended.get(consumer);
  if (!suspension || --suspension.count > 0) return;
  suspended.delete(consumer);
  suspensionsChanged();
}

/** Whether the page shows no video of `consumer` for now. */
export function nativeVideoSuspended(consumer: number): boolean {
  return suspended.has(consumer);
}

export function subscribeNativeVideoSuspensions(
  listener: () => void,
): () => void {
  suspensionListeners.add(listener);
  return () => suspensionListeners.delete(listener);
}

/** Tests: a page of their own (`undefined`: the real one), nothing open.
 * `bytes`: as if a frame had already arrived as bytes. */
export function resetNativeVideoForTests(
  replacement?: NativeVideoHost,
  bytes = false,
): void {
  for (const feed of [...feeds.values()]) void feed.close();
  host = replacement ?? page;
  limit = null;
  limitListeners.clear();
  suspended.clear();
  suspensionListeners.clear();
  carried = bytes;
  left = 0;
}
