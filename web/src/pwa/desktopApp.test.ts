import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearToasts, useToasts } from "../components/toasts.ts";
import {
  ARCH_INSTALL_COMMAND,
  desktopPlatform,
  LINUX_TARBALL_URL,
  offerDesktopApp,
  WINDOWS_SETUP_URL,
} from "./desktopApp.ts";
import { trackInstallation, useInstallation } from "./install.ts";

const UA = {
  windows:
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
  linux:
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
  firefoxLinux:
    "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0",
  armLinux:
    "Mozilla/5.0 (X11; Linux aarch64; rv:140.0) Gecko/20100101 Firefox/140.0",
  mac: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
  android:
    "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36",
  chromeos:
    "Mozilla/5.0 (X11; CrOS x86_64 14541.0.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36",
};

describe("desktop platform", () => {
  it("offers Windows and x64 Linux only", () => {
    expect(desktopPlatform({ userAgent: UA.windows })).toBe("windows");
    expect(desktopPlatform({ userAgent: UA.linux })).toBe("linux");
    expect(desktopPlatform({ userAgent: UA.firefoxLinux })).toBe("linux");
    for (const ua of [UA.armLinux, UA.mac, UA.android, UA.chromeos]) {
      expect(desktopPlatform({ userAgent: ua })).toBeNull();
    }
  });

  it("prefers the client hint, which survives a reduced user agent", () => {
    const hinted = (platform: string, mobile = false) =>
      desktopPlatform({
        userAgent: UA.linux,
        userAgentData: { platform, mobile },
      });
    expect(hinted("Windows")).toBe("windows");
    expect(hinted("Linux")).toBe("linux");
    expect(hinted("Chrome OS")).toBeNull();
    expect(hinted("macOS")).toBeNull();
    expect(hinted("Android", true)).toBeNull();
  });

  it("is part of the install state, never on phones or in the Tauri app", () => {
    const browser = (userAgent: string) =>
      Object.assign(new EventTarget(), {
        isSecureContext: true,
        navigator: { userAgent, platform: "", maxTouchPoints: 0 },
        matchMedia: () => Object.assign(new EventTarget(), { matches: false }),
      }) as unknown as Window;
    const desktop = (userAgent: string, native = false) => {
      trackInstallation(browser(userAgent), native)();
      return useInstallation.getState().desktop;
    };
    expect(desktop(UA.windows)).toBe("windows");
    expect(desktop(UA.linux)).toBe("linux");
    expect(desktop(UA.android)).toBeNull();
    expect(desktop(UA.linux, true)).toBeNull();
  });

  it("links the newest release's packages and the published install script", () => {
    const latest =
      "https://github.com/Till-und-Scholler-AI-slop/Gelabber/releases/latest/download/";
    expect(WINDOWS_SETUP_URL).toBe(
      `${latest}gelabber-native-windows-x64-setup.exe`,
    );
    expect(LINUX_TARBALL_URL).toBe(`${latest}gelabber-native-linux-x64.tar.gz`);
    expect(ARCH_INSTALL_COMMAND).toBe(
      "curl -fsSL https://till-und-scholler-ai-slop.github.io/Gelabber/install.sh | sudo sh",
    );
  });
});

describe("desktop app offer", () => {
  let stored: Map<string, string>;
  beforeEach(() => {
    stored = new Map();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => void stored.set(key, value),
    });
  });
  afterEach(() => {
    clearToasts();
    vi.unstubAllGlobals();
  });

  it("tells a browser once and opens the steps from its button", () => {
    const open = vi.fn();
    expect(offerDesktopApp("linux", open)).toBe(true);
    const [toast] = useToasts.getState().toasts;
    expect(toast?.message).toBe(
      "Gelabber gibt es auch als Desktop-App für Linux.",
    );
    toast?.action?.onClick();
    expect(open).toHaveBeenCalledOnce();
    clearToasts();
    expect(offerDesktopApp("linux", open)).toBe(false);
    expect(useToasts.getState().toasts).toEqual([]);
  });

  it("stays quiet without a desktop app or where nothing can be remembered", () => {
    expect(offerDesktopApp(null, vi.fn())).toBe(false);
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new DOMException("blocked", "SecurityError");
      },
    });
    expect(offerDesktopApp("windows", vi.fn())).toBe(false);
    expect(useToasts.getState().toasts).toEqual([]);
  });
});
