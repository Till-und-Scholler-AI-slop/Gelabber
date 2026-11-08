// The Tauri app (up to 0.8.x) still loads this client from the server; 0.9's
// native app replaced it. The client only tells it so and keeps it usable.

import { ERROR_MS, notify } from "./components/toasts.ts";
import { APP_VERSION } from "./version.ts";

type TauriInternals = {
  invoke(command: string, args?: Record<string, unknown>): Promise<unknown>;
};

function internals(): TauriInternals | null {
  if (typeof window === "undefined") return null;
  const value = (window as unknown as { __TAURI_INTERNALS__?: unknown })
    .__TAURI_INTERNALS__;
  // `typeof null` is "object" too.
  return typeof value === "object" && value !== null
    ? (value as TauriInternals)
    : null;
}

/** True inside the Tauri app. */
export function isTauriApp(): boolean {
  return internals() !== null;
}

/** Marks the Tauri app's `<html data-tauri>`: its WebKitGTK ignores mouse
 * wheels under the root's overscroll guard (index.css). */
export function markTauriRoot(root: HTMLElement): void {
  if (isTauriApp()) root.dataset.tauri = "";
}

/** Compares dotted numbers part by part; a missing part counts as 0. */
export function olderVersion(app: string, web: string): boolean {
  const a = app.split(".").map(Number);
  const b = web.split(".").map(Number);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] || 0;
    const y = b[i] || 0;
    if (x !== y) return x < y;
  }
  return false;
}

/** The app's version from its `media_info`; null when it sends none. */
async function appVersion(app: TauriInternals): Promise<string | null> {
  try {
    const info = await app.invoke("media_info");
    const version = (info as { version?: unknown } | null)?.version;
    return typeof version === "string" && version ? version : null;
  } catch {
    return null;
  }
}

let warned = false;

/** At most once per page; never in a browser. An app that sends no version
 * (older than 0.6, or one that cannot answer) counts as outdated. */
export async function warnIfTauriOutdated(): Promise<void> {
  const app = internals();
  if (warned || !app) return;
  const version = await appVersion(app);
  if (version !== null && !olderVersion(version, APP_VERSION)) return;
  if (warned) return;
  warned = true;
  const versions =
    version === null ? "" : ` (v${version}, Server v${APP_VERSION})`;
  notify(
    `Die Desktop-App ist veraltet${versions}. Ab 0.9 gibt es eine neue Desktop-App: unter Windows ab v0.7 über das App-Update, unter Arch mit pacman -Syu, sonst gelabber-native-* aus dem GitHub-Release.`,
    { duration: ERROR_MS },
  );
}
