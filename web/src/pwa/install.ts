import { create } from "zustand";

import { desktopPlatform, type DesktopPlatform } from "./desktopApp.ts";

type InstallChoice = { outcome: "accepted" | "dismissed" };
export interface InstallPromptEvent extends Event {
  prompt(): Promise<void>;
  userChoice: Promise<InstallChoice>;
}

export type InstallState = {
  secure: boolean;
  /** Installed as far as this page can tell: running as the app, or just set up. */
  installed: boolean;
  /** This window is the installed app, not a browser tab. */
  standalone: boolean;
  ios: boolean;
  /** A phone or tablet: installing means the home screen, not an app window. */
  mobile: boolean;
  /** The system whose desktop app this browser could run instead; null on
   * phones and other systems. */
  desktop: DesktopPlatform | null;
  /** Which desktop browser this is, for the install hint; "other" on phones
   * and where the user agent says nothing we know. */
  browser: DesktopBrowser;
  available: boolean;
  pending: boolean;
  accepted: boolean;
  failed: boolean;
};

const initial: InstallState = {
  secure: false,
  installed: false,
  standalone: false,
  ios: false,
  mobile: false,
  desktop: null,
  browser: "other",
  available: false,
  pending: false,
  accepted: false,
  failed: false,
};

export type DesktopBrowser = "chromium" | "safari" | "firefox" | "other";

/**
 * The desktop browser family, as far as the user agent tells: Chromium
 * (Chrome, Edge) shows an install icon in the address bar, Safari on macOS
 * has File › Add to Dock, Firefox installs no web apps. Anything else gets
 * the generic hint.
 */
export function desktopBrowser(nav: {
  userAgent: string;
  userAgentData?: { brands?: { brand: string }[] };
}): DesktopBrowser {
  const ua = nav.userAgent;
  // Opera has no install icon in the address bar.
  if (/OPR\/|Opera/.test(ua)) return "other";
  if (
    nav.userAgentData?.brands?.some((entry) => entry.brand === "Chromium") ||
    /Chrome\/|Chromium\/|Edg\//.test(ua)
  )
    return "chromium";
  if (/Firefox\//.test(ua)) return "firefox";
  if (/Macintosh/.test(ua) && /Version\/[\d.]+.*Safari\//.test(ua))
    return "safari";
  return "other";
}

export const useInstallation = create<InstallState>(() => initial);
let deferredPrompt: InstallPromptEvent | null = null;
let generation = 0;

/** Bind before React renders: Chrome may offer installation before settings open. */
export function trackInstallation(target: Window): () => void {
  const current = ++generation;
  deferredPrompt = null;
  const display = target.matchMedia("(display-mode: standalone)");
  const nav = target.navigator as Navigator & {
    standalone?: boolean;
    userAgentData?: {
      platform?: string;
      mobile?: boolean;
      brands?: { brand: string }[];
    };
  };
  const standalone = () => display.matches || nav.standalone === true;
  const ios =
    /iPad|iPhone|iPod/.test(nav.userAgent) ||
    (nav.platform === "MacIntel" && nav.maxTouchPoints > 1);
  const mobile = ios || /Android/.test(nav.userAgent);
  useInstallation.setState({
    ...initial,
    secure: target.isSecureContext,
    installed: standalone(),
    standalone: standalone(),
    ios,
    mobile,
    desktop: mobile ? null : desktopPlatform(nav),
    browser: mobile ? "other" : desktopBrowser(nav),
  });

  const beforeInstall = (event: Event) => {
    if (!target.isSecureContext || useInstallation.getState().installed) return;
    event.preventDefault();
    deferredPrompt = event as InstallPromptEvent;
    useInstallation.setState({
      available: true,
      accepted: false,
      failed: false,
    });
  };
  const installed = () => {
    deferredPrompt = null;
    useInstallation.setState({
      installed: true,
      available: false,
      pending: false,
      accepted: false,
      failed: false,
    });
  };
  const refresh = () => {
    if (!standalone()) return;
    installed();
    useInstallation.setState({ standalone: true });
  };
  target.addEventListener("beforeinstallprompt", beforeInstall);
  target.addEventListener("appinstalled", installed);
  target.addEventListener("pageshow", refresh);
  display.addEventListener("change", refresh);
  return () => {
    target.removeEventListener("beforeinstallprompt", beforeInstall);
    target.removeEventListener("appinstalled", installed);
    target.removeEventListener("pageshow", refresh);
    display.removeEventListener("change", refresh);
    if (generation === current) {
      generation++;
      deferredPrompt = null;
    }
  };
}

/**
 * Whether a page that is about something else (login) should advertise
 * installation. A phone can always be told how, or that HTTPS is missing. A
 * desktop browser is only worth the space while it actually offers a prompt.
 */
export function installHintUseful(state: InstallState): boolean {
  if (state.standalone) return false;
  // `installed` without `standalone` is the confirmation right after setup.
  if (state.installed || state.mobile) return true;
  return (
    state.secure &&
    (state.available || state.pending || state.accepted || state.failed)
  );
}

/** Called directly from a button click to preserve the browser's user gesture. */
export async function promptInstallation(): Promise<void> {
  const event = deferredPrompt;
  if (!event || useInstallation.getState().pending) return;
  const current = generation;
  deferredPrompt = null; // A browser install prompt is single-use, even on dismissal.
  useInstallation.setState({ pending: true, available: false, failed: false });
  try {
    await event.prompt();
    const choice = await event.userChoice;
    if (current === generation && !useInstallation.getState().installed) {
      // Acceptance isn't installation: only appinstalled/standalone confirms that.
      useInstallation.setState({ accepted: choice.outcome === "accepted" });
    }
  } catch {
    if (current === generation && !useInstallation.getState().installed) {
      useInstallation.setState({ failed: true });
    }
  } finally {
    if (current === generation) useInstallation.setState({ pending: false });
  }
}
