// Remote video in a native viewer window of the desktop app. Apps up to
// 0.5.x have nothing else: their webview cannot show the native core's video.
// Newer apps draw it in the page (videoFeed.ts) and keep the window as an
// extra. Which consumers have a window is shared by every tile showing the
// same stream.
import { invokeNative, nativeBridge } from "./bridge.ts";
import { isNativeTrack, type NativeTrack } from "./tracks.ts";

/** Open windows by consumer: the track they show and the shown height. */
const open = new Map<number, { trackId: string; height: number }>();
const listeners = new Set<() => void>();
let version = 0;

function changed(): void {
  version++;
  for (const listener of listeners) listener();
}

export function subscribeNativeViewers(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Changes whenever a viewer opens or closes (useSyncExternalStore). */
export function nativeViewersVersion(): number {
  return version;
}

/** The live remote video track of a stream in the desktop app, if any. */
export function nativeVideoTrack(
  stream: MediaStream | null,
): NativeTrack | null {
  const track = stream?.getVideoTracks()[0];
  return isNativeTrack(track) &&
    track.readyState === "live" &&
    track.handle.consumer !== undefined
    ? track
    : null;
}

/** Height a viewer window shows the track at, in physical pixels; 0 when
 * no window shows it (the server then sends the low layer). */
export function nativeViewerHeight(trackId: string): number {
  let height = 0;
  for (const view of open.values())
    if (view.trackId === trackId) height = Math.max(height, view.height);
  return height;
}

export function nativeViewerOpen(consumer: number): boolean {
  return open.has(consumer);
}

export async function openNativeViewer(
  track: NativeTrack,
  title: string,
): Promise<void> {
  const consumer = track.handle.consumer;
  if (consumer === undefined || open.has(consumer)) return;
  const view = { trackId: track.id, height: 0 };
  open.set(consumer, view);
  changed();
  const events = await nativeBridge().channel<{
    type: string;
    height?: number;
  }>((event) => {
    if (event.type === "closed") closeNativeViewer(consumer);
    else if (event.type === "height" && typeof event.height === "number")
      view.height = event.height;
  });
  try {
    await invokeNative("media_viewer_open", { consumer, title, events });
  } catch (error) {
    closeNativeViewer(consumer);
    throw error;
  }
}

export function closeNativeViewer(consumer: number): void {
  if (!open.delete(consumer)) return;
  changed();
  void invokeNative("media_viewer_close", { consumer }).catch(() => undefined);
}

/** Tests: forget open viewers. */
export function resetNativeViewersForTests(): void {
  open.clear();
  changed();
}
