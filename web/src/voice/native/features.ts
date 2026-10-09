// What the desktop app around this page can do, from `media_info().features`.
// The web client is served by the server, so it also runs in older apps:
// a feature counts only once the app has named it.
import { invokeNative, isDesktopApp } from "./bridge.ts";

let features: ReadonlySet<string> = new Set();
let asked = false;
const listeners = new Set<() => void>();

function ask(): void {
  if (asked || !isDesktopApp()) return;
  asked = true;
  invokeNative<{ features?: unknown } | null>("media_info")
    .then((info) => (Array.isArray(info?.features) ? info.features : []))
    // An app without the command has no features.
    .catch(() => [])
    .then((names: unknown[]) => {
      features = new Set(
        names.filter((name): name is string => typeof name === "string"),
      );
      for (const listener of [...listeners]) listener();
    });
}

/** Whether the app named `name`; false until `media_info` answered and in
 * a browser. */
export function hasNativeFeature(name: string): boolean {
  ask();
  return features.has(name);
}

/** For useSyncExternalStore: runs when the answer arrived. */
export function subscribeNativeFeatures(listener: () => void): () => void {
  ask();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

// Ask at load in the app, so the answer is there before the first tile.
ask();
