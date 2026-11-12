// The native desktop app (Windows, Linux x64): which one this browser's
// system takes, where its newest release is, and the one-time hint after
// signing in that points to Settings → App.

import { ERROR_MS, notify } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { pwa } from "../i18n/catalogs/pwa.ts";

export type DesktopPlatform = "windows" | "linux";

export const DESKTOP_PLATFORM_LABEL: Record<DesktopPlatform, string> = {
  windows: "Windows",
  linux: "Linux",
};

// Always the newest release: GitHub redirects `latest/download/<asset>`.
const LATEST =
  "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest/download";
export const WINDOWS_SETUP_URL = `${LATEST}/gelabber-native-windows-x64-setup.exe`;
export const LINUX_TARBALL_URL = `${LATEST}/gelabber-native-linux-x64.tar.gz`;
/** desktop/packaging/arch/install.sh, published with the pacman repository. */
export const ARCH_INSTALL_COMMAND =
  "curl -fsSL https://till-und-scholler-ai-slop.github.io/Gelabber/install.sh | sudo sh";

type PlatformNavigator = {
  userAgent: string;
  userAgentData?: { platform?: string; mobile?: boolean };
};

/**
 * The system whose desktop app this browser can run, or null: macOS, phones,
 * tablets, ChromeOS and ARM Linux have none (the builds are x64).
 */
export function desktopPlatform(
  nav: PlatformNavigator,
): DesktopPlatform | null {
  if (nav.userAgentData?.mobile) return null;
  const hinted = nav.userAgentData?.platform ?? "";
  const ua = nav.userAgent;
  if (/Android|CrOS|Chrome ?OS|iPhone|iPad/i.test(`${hinted} ${ua}`))
    return null;
  if (hinted === "Windows" || (!hinted && /Windows NT/.test(ua)))
    return "windows";
  if (hinted === "Linux" || (!hinted && /Linux/.test(ua))) {
    return /aarch64|arm/i.test(ua) ? null : "linux";
  }
  return null;
}

const OFFERED_KEY = "gelabber.desktopApp.offered";

/**
 * Tells this browser once that the desktop app exists; `open` shows it
 * (Settings → App). Where the browser keeps nothing, it stays quiet rather
 * than repeat itself on every visit.
 */
export function offerDesktopApp(
  platform: DesktopPlatform | null,
  open: () => void,
): boolean {
  if (!platform) return false;
  try {
    if (localStorage.getItem(OFFERED_KEY) !== null) return false;
    localStorage.setItem(OFFERED_KEY, "1");
  } catch {
    return false;
  }
  notify(
    translate(pwa, "offer.text", {
      platform: DESKTOP_PLATFORM_LABEL[platform],
    }),
    {
      action: { label: translate(pwa, "offer.view"), onClick: open },
      duration: ERROR_MS,
    },
  );
  return true;
}
