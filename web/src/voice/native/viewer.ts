// Remote video in a native viewer window of the desktop app. Apps up to
// 0.5.x have nothing else: their webview cannot show the native core's video.
// Newer apps draw it in the page (videoFeed.ts) and keep the window as an
// extra. A stream is in its window or in the page, not in both: next to a
// window the app hands the page's views of the stream the window's frames,
// unscaled, whatever size the page asked for. Which consumers have a window
// is shared by every tile showing the same stream.
import { invokeNative, nativeBridge } from "./bridge.ts";
import { isNativeTrack, type NativeTrack } from "./tracks.ts";
import {
  nativeCanvasHeight,
  resumeNativeVideo,
  suspendNativeVideo,
} from "./videoFeed.ts";

/** Open windows by consumer: the track they show and the shown height. */
const open = new Map<number, { trackId: string; height: number }>();
const listeners = new Set<() => void>();
let version = 0;
/** The app's answer to the last window command of a consumer. */
const answered = new Map<number, Promise<void>>();

function changed(): void {
  version++;
  for (const listener of listeners) listener();
}

/** Runs `command` once the app has answered the consumer's window commands
 * before it. The app works on commands side by side: a close that overtook
 * its open would leave it feeding a window the page knows nothing of. */
function inTurn<T>(consumer: number, command: () => Promise<T>): Promise<T> {
  const result = (answered.get(consumer) ?? Promise.resolve()).then(command);
  const settled = result.then(
    () => undefined,
    () => undefined,
  );
  answered.set(consumer, settled);
  void settled.then(() => {
    if (answered.get(consumer) === settled) answered.delete(consumer);
  });
  return result;
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
 * no window shows it (the server then sends the low layer). A window that
 * has not said yet counts with the height the page showed the track at. */
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
  // The layer the page's canvases asked for stays until the window reports
  // its own height: the stream does not drop to the low layer in between.
  const view = { trackId: track.id, height: nativeCanvasHeight(track.id) };
  open.set(consumer, view);
  const suspended = suspendNativeVideo(consumer);
  changed();
  try {
    await inTurn(consumer, async () => {
      // Not before the app has closed the page's views of the stream: one
      // that is still open, or waits for its next frame, gets the window's.
      await suspended;
      const events = await nativeBridge().channel<{
        type: string;
        height?: number;
      }>((event) => {
        if (event.type === "closed") {
          if (open.get(consumer) === view) closeNativeViewer(consumer);
        } else if (event.type === "height" && typeof event.height === "number")
          view.height = event.height;
      });
      // Closed again in the meantime: there is nothing to open any more.
      if (open.get(consumer) !== view) return;
      await invokeNative("media_viewer_open", { consumer, title, events });
    });
  } catch (error) {
    if (open.get(consumer) === view) closeNativeViewer(consumer);
    throw error;
  }
}

export function closeNativeViewer(consumer: number): void {
  if (!open.delete(consumer)) return;
  changed();
  // The page shows the stream again when the app no longer feeds a window:
  // a view that opens earlier gets the window's frames.
  void inTurn(consumer, () => invokeNative("media_viewer_close", { consumer }))
    // The window went with whatever kept the app from answering.
    .catch(() => undefined)
    .then(() => resumeNativeVideo(consumer));
}

/** Tests: forget open viewers. */
export function resetNativeViewersForTests(): void {
  for (const consumer of open.keys()) resumeNativeVideo(consumer);
  open.clear();
  answered.clear();
  changed();
}
