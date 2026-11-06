// A desktop app older than the web client it loads. From 0.9 every such app
// is the Tauri app, which 0.9 replaces; say once where its successor comes from.

import { ERROR_MS, notify } from "../../components/toasts.ts";
import { APP_VERSION } from "../../version.ts";
import { isDesktopApp } from "./bridge.ts";
import { loadNativeFeatures, nativeAppVersion } from "./features.ts";

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

let warned = false;

/** At most once per page; never in a browser. An app that sends no version
 * (older than 0.6, or one that cannot answer) counts as outdated. */
export async function warnIfDesktopOutdated(): Promise<void> {
  if (warned || !isDesktopApp()) return;
  await loadNativeFeatures();
  const app = nativeAppVersion();
  if (app !== null && !olderVersion(app, APP_VERSION)) return;
  if (warned) return;
  warned = true;
  const versions = app === null ? "" : ` (v${app}, Server v${APP_VERSION})`;
  notify(
    `Die Desktop-App ist veraltet${versions}. Ab 0.9 gibt es eine neue Desktop-App: unter Windows über das App-Update, unter Arch mit pacman -Syu, sonst gelabber-native-* aus dem GitHub-Release.`,
    { duration: ERROR_MS },
  );
}
