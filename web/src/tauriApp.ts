// The Tauri app (up to 0.8.x) still loads this client from the server; 0.9's
// native app replaced it. The client only tells it so and keeps it usable.

import { ERROR_MS, notify } from "./components/toasts.ts";

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

let warned = false;

/** At most once per page; never in a browser. Any Tauri app is 0.8.x or
 * older, so it is always outdated. */
export function warnIfTauriOutdated(): void {
  if (warned || !isTauriApp()) return;
  warned = true;
  notify(
    "Die Desktop-App ist veraltet. Ab 0.9 gibt es eine neue Desktop-App: unter Windows ab v0.7 über das App-Update, unter Arch mit pacman -Syu, sonst gelabber-native-* aus dem GitHub-Release.",
    { duration: ERROR_MS },
  );
}
