// What the installed desktop app can do.
//
// The server serves one web client to every installed app, old and new, on
// every platform. So the client asks instead of assuming: `media_info`
// answers with a `features` list. Apps up to 0.5.x answer without one; they
// are the Linux app with screen capture, camera and application sound.

import { isDesktopApp, nativeBridge, type NativeBridge } from "./bridge.ts";

/** Names in `media_info().features` (desktop/app/src/media.rs). */
export type NativeFeature =
  /** Screen and window capture. */
  | "screen"
  | "camera"
  /** Sound of other applications for a share. */
  | "app-audio"
  /** "Every application" leaves out Gelabber's own playout, so a share can
   * carry sound without sending the call back into it. */
  | "app-audio-excludes-self"
  /** The page gets video frames and draws them itself. */
  | "video-frames";

export type NativeFeatures = ReadonlySet<string>;

const LEGACY: NativeFeatures = new Set(["screen", "camera", "app-audio"]);
const NONE: NativeFeatures = new Set();

function read(info: unknown): NativeFeatures {
  const list = (info as { features?: unknown } | null | undefined)?.features;
  if (!Array.isArray(list)) return LEGACY;
  return new Set(
    list.filter((name): name is string => typeof name === "string"),
  );
}

type Asked = {
  bridge: NativeBridge;
  features: NativeFeatures | null;
  answered: Promise<NativeFeatures>;
};

let asked: Asked | null = null;
const listeners = new Set<() => void>();

/** One question per page. It is kept per bridge only so that a test's next
 * fake app is asked again. */
function ask(): Asked | null {
  if (!isDesktopApp()) return null;
  const bridge = nativeBridge();
  if (asked?.bridge === bridge) return asked;
  const mine: Asked = {
    bridge,
    features: null,
    answered: Promise.resolve()
      .then(() => bridge.invoke<unknown>("media_info"))
      .then(read)
      // An app that cannot answer is no newer than those without a list.
      .catch(() => LEGACY)
      .then((features) => {
        mine.features = features;
        if (asked === mine) for (const listener of [...listeners]) listener();
        return features;
      }),
  };
  asked = mine;
  return mine;
}

/** The app's features: `null` until it has answered, empty in a browser. */
export function nativeFeatures(): NativeFeatures | null {
  const current = ask();
  return current ? current.features : NONE;
}

/** The same once the app has answered; at once in a browser. */
export function loadNativeFeatures(): Promise<NativeFeatures> {
  return ask()?.answered ?? Promise.resolve(NONE);
}

/** For `useSyncExternalStore`, with `nativeFeatures` as the snapshot. */
export function subscribeNativeFeatures(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** False in a browser, and in the app until it has answered. */
export function hasNativeFeature(name: NativeFeature): boolean {
  return nativeFeatures()?.has(name) ?? false;
}

// Asked while the page loads, so the answer is there before a control renders.
ask();
