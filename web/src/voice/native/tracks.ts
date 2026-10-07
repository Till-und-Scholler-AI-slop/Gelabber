// Stand-ins for MediaStreamTrack, MediaStream and the audio element when the
// desktop app's native core owns capture and playout. The session keeps its
// track-keyed bookkeeping; these objects carry the native handles instead of
// media. WebKitGTK cannot play them, so nothing here touches the DOM.
import { invokeNative, isDesktopApp } from "./bridge.ts";

let nextId = 0;
const uid = (prefix: string) =>
  `${prefix}-${Date.now().toString(36)}-${(++nextId).toString(36)}`;

function report(error: unknown): void {
  console.warn("[gelabber] native media", error);
}

export type NativeHandle = {
  /** Local capture source (microphone, screen) owned by the track. */
  source?: number;
  /** Remote consumer the track plays. */
  consumer?: number;
};

export class NativeTrack extends EventTarget {
  readonly id = uid("native-track");
  readonly native = true;
  readonly handle: NativeHandle;
  readonly kind: "audio" | "video";
  readonly label: string;
  contentHint = "";
  muted = false;
  readyState: MediaStreamTrackState = "live";
  onended: ((event: Event) => void) | null = null;
  onmute: ((event: Event) => void) | null = null;
  onunmute: ((event: Event) => void) | null = null;
  private on = true;
  private settings: MediaTrackSettings;
  private stopHooks: Array<() => void> = [];

  constructor(
    kind: "audio" | "video",
    label: string,
    handle: NativeHandle = {},
    settings: MediaTrackSettings = {},
  ) {
    super();
    this.kind = kind;
    this.label = label;
    this.handle = handle;
    this.settings = settings;
  }

  get enabled(): boolean {
    return this.on;
  }
  set enabled(value: boolean) {
    if (this.on === value) return;
    this.on = value;
    const source = this.handle.source;
    if (source !== undefined && this.readyState === "live")
      void invokeNative("media_source_set_enabled", {
        source,
        enabled: value,
      }).catch(report);
  }
  getSettings(): MediaTrackSettings {
    return { ...this.settings };
  }
  updateSettings(next: MediaTrackSettings): void {
    this.settings = { ...this.settings, ...next };
  }
  getConstraints(): MediaTrackConstraints {
    return {};
  }
  getCapabilities(): MediaTrackCapabilities {
    return {};
  }
  /** Native capture keeps its own profile; constraints are accepted as is. */
  async applyConstraints(constraints?: MediaTrackConstraints): Promise<void> {
    void constraints;
  }
  clone(): never {
    throw new DOMException(
      "native tracks cannot be cloned",
      "NotSupportedError",
    );
  }
  onStop(hook: () => void): void {
    this.stopHooks.push(hook);
  }
  /** Like MediaStreamTrack.stop(): no `ended` event, the source is released. */
  stop(): void {
    if (this.readyState === "ended") return;
    this.readyState = "ended";
    this.release();
  }
  /** The source went away on its own (screen share stopped in the desktop). */
  end(): void {
    if (this.readyState === "ended") return;
    this.readyState = "ended";
    this.release();
    const event = new Event("ended");
    this.dispatchEvent(event);
    this.onended?.(event);
  }
  private release(): void {
    for (const hook of this.stopHooks.splice(0)) hook();
    const source = this.handle.source;
    if (source !== undefined)
      void invokeNative("media_source_close", { source }).catch(report);
  }
}

export function isNativeTrack(track: unknown): track is NativeTrack {
  return track instanceof NativeTrack;
}

export class NativeStream extends EventTarget {
  readonly id = uid("native-stream");
  readonly native = true;
  private tracks: NativeTrack[] = [];
  onaddtrack: ((event: Event) => void) | null = null;
  onremovetrack: ((event: Event) => void) | null = null;

  constructor(tracks: Iterable<MediaStreamTrack> = []) {
    super();
    for (const track of tracks) this.addTrack(track);
  }
  get active(): boolean {
    return this.tracks.some((track) => track.readyState === "live");
  }
  getTracks(): MediaStreamTrack[] {
    return [...this.tracks] as unknown as MediaStreamTrack[];
  }
  getAudioTracks(): MediaStreamTrack[] {
    return this.tracks.filter(
      (track) => track.kind === "audio",
    ) as unknown as MediaStreamTrack[];
  }
  getVideoTracks(): MediaStreamTrack[] {
    return this.tracks.filter(
      (track) => track.kind === "video",
    ) as unknown as MediaStreamTrack[];
  }
  getTrackById(id: string): MediaStreamTrack | null {
    return (this.tracks.find((track) => track.id === id) ??
      null) as unknown as MediaStreamTrack | null;
  }
  addTrack(track: MediaStreamTrack): void {
    if (!isNativeTrack(track))
      throw new TypeError("only native tracks fit a native stream");
    if (this.tracks.includes(track)) return;
    this.tracks.push(track);
    this.notify("addtrack", track);
  }
  removeTrack(track: MediaStreamTrack): void {
    const index = this.tracks.indexOf(track as unknown as NativeTrack);
    if (index < 0) return;
    this.tracks.splice(index, 1);
    this.notify("removetrack", track as unknown as NativeTrack);
  }
  clone(): never {
    throw new DOMException(
      "native streams cannot be cloned",
      "NotSupportedError",
    );
  }
  private notify(type: "addtrack" | "removetrack", track: NativeTrack): void {
    const event = Object.assign(new Event(type), { track });
    this.dispatchEvent(event);
    (type === "addtrack" ? this.onaddtrack : this.onremovetrack)?.(event);
  }
}

export function isNativeStream(stream: unknown): stream is NativeStream {
  return stream instanceof NativeStream;
}

let outputDevice: string | null = null;
let outputChange: Promise<void> = Promise.resolve();

/** The native engine has one playout device for every consumer. */
function selectOutput(id: string): Promise<void> {
  outputChange = outputChange
    .catch(() => undefined)
    .then(async () => {
      if (outputDevice === id) return;
      await invokeNative("media_audio_configure", { options: { output: id } });
      outputDevice = id;
    });
  return outputChange;
}

/**
 * The part of HTMLAudioElement the session uses, applied to native consumers:
 * the element's volume (0 while paused, muted or detached) becomes each
 * attached consumer's playout volume. Native consumers start silent.
 */
export class NativeAudioOutput {
  readonly native = true;
  autoplay = false;
  paused = true;
  private attributes = new Map<string, string>();
  private stream: NativeStream | null = null;
  private level = 1;
  private silenced = false;
  private applied = new Map<number, number>();
  private readonly listener = () => this.apply();

  get srcObject(): MediaStream | null {
    return this.stream as unknown as MediaStream | null;
  }
  set srcObject(value: MediaStream | null) {
    if ((value as unknown) === this.stream) return;
    if (value !== null && !isNativeStream(value))
      throw new TypeError("native audio output plays native streams only");
    this.stream?.removeEventListener("addtrack", this.listener);
    this.stream?.removeEventListener("removetrack", this.listener);
    this.stream = value;
    this.stream?.addEventListener("addtrack", this.listener);
    this.stream?.addEventListener("removetrack", this.listener);
    this.apply();
  }
  get volume(): number {
    return this.level;
  }
  set volume(value: number) {
    this.level = Math.min(1, Math.max(0, value));
    this.apply();
  }
  get muted(): boolean {
    return this.silenced;
  }
  set muted(value: boolean) {
    this.silenced = value;
    this.apply();
  }
  setAttribute(name: string, value: string): void {
    this.attributes.set(name, value);
  }
  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }
  removeAttribute(name: string): void {
    this.attributes.delete(name);
  }
  async play(): Promise<void> {
    this.paused = false;
    this.apply();
  }
  pause(): void {
    this.paused = true;
    this.apply();
  }
  setSinkId(id: string): Promise<void> {
    return selectOutput(id);
  }
  private apply(): void {
    const volume =
      this.paused || this.silenced || !this.stream ? 0 : this.level;
    const wanted = new Map<number, number>();
    for (const track of this.stream?.getAudioTracks() ?? []) {
      const consumer = (track as unknown as NativeTrack).handle.consumer;
      if (consumer !== undefined && track.readyState === "live")
        wanted.set(consumer, volume);
    }
    // Detached consumers fall silent.
    for (const consumer of this.applied.keys())
      if (!wanted.has(consumer)) wanted.set(consumer, 0);
    for (const [consumer, value] of wanted) {
      // Native consumers start silent, so unknown means 0.
      if ((this.applied.get(consumer) ?? 0) === value) continue;
      if (value === 0) this.applied.delete(consumer);
      else this.applied.set(consumer, value);
      void invokeNative("media_consumer_set_volume", {
        consumer,
        volume: value,
      }).catch(() => {
        // The consumer closed in the meantime.
      });
    }
  }
}

/** `new MediaStream(tracks)` that also holds native tracks in the desktop app. */
export function createStream(
  tracks: MediaStreamTrack[] = [],
): MediaStream | null {
  if (isDesktopApp()) return new NativeStream(tracks) as unknown as MediaStream;
  if (typeof MediaStream === "undefined") return null;
  return new MediaStream(tracks);
}

/** `new Audio()`, or a native output in the desktop app. */
export function createAudioOutput(): HTMLAudioElement | null {
  if (isDesktopApp())
    return new NativeAudioOutput() as unknown as HTMLAudioElement;
  if (typeof Audio === "undefined") return null;
  return new Audio();
}

/** Tests: forget the cached output device. */
export function resetNativeOutputForTests(): void {
  outputDevice = null;
  outputChange = Promise.resolve();
}
