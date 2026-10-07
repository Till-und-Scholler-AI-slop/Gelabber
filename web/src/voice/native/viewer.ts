// Remote video in the desktop app: the webview cannot show the native core's
// video, so a stream opens in a native viewer window. Which consumers have a
// window is shared by every tile showing the same stream.
import { invokeNative, nativeBridge } from "./bridge.ts";
import { isNativeTrack } from "./tracks.ts";

const open = new Set<number>();
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

/** The native consumer behind a stream's video, if any. */
export function nativeVideoConsumer(stream: MediaStream | null): number | null {
  const track = stream?.getVideoTracks()[0];
  return isNativeTrack(track) && track.readyState === "live"
    ? (track.handle.consumer ?? null)
    : null;
}

export function nativeViewerOpen(consumer: number): boolean {
  return open.has(consumer);
}

export async function openNativeViewer(
  consumer: number,
  title: string,
): Promise<void> {
  if (open.has(consumer)) return;
  open.add(consumer);
  changed();
  const events = await nativeBridge().channel<{ type: string }>((event) => {
    if (event.type === "closed") closeNativeViewer(consumer);
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
