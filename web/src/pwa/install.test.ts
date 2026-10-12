import { afterEach, describe, expect, it, vi } from "vitest";
import {
  installHintUseful,
  promptInstallation,
  trackInstallation,
  useInstallation,
} from "./install.ts";

class Browser extends EventTarget {
  isSecureContext = true;
  navigator = {
    userAgent: "Android",
    platform: "Linux",
    maxTouchPoints: 1,
    standalone: false,
  };
  display = Object.assign(new EventTarget(), { matches: false });
  matchMedia = () => this.display;
}

let cleanup: (() => void) | undefined;
afterEach(() => cleanup?.());
function track(browser = new Browser(), native = false) {
  cleanup = trackInstallation(browser as unknown as Window, native);
  return browser;
}
function offer(
  browser: Browser,
  outcome: "accepted" | "dismissed" = "accepted",
) {
  const prompt = vi.fn().mockResolvedValue(undefined);
  const event = Object.assign(
    new Event("beforeinstallprompt", { cancelable: true }),
    {
      prompt,
      userChoice: Promise.resolve({ outcome }),
    },
  );
  browser.dispatchEvent(event);
  return event;
}

describe("browser installation", () => {
  it("retains an early browser offer without automatically prompting", async () => {
    const event = offer(track());
    expect(event.defaultPrevented).toBe(true);
    expect(event.prompt).not.toHaveBeenCalled();
    expect(useInstallation.getState().available).toBe(true);
    await promptInstallation();
    expect(event.prompt).toHaveBeenCalledOnce();
    expect(useInstallation.getState()).toMatchObject({
      accepted: true,
      installed: false,
      pending: false,
      available: false,
    });
  });

  it("uses a dismissed prompt only once and accepts a later fresh offer", async () => {
    const browser = track();
    const first = offer(browser, "dismissed");
    await promptInstallation();
    await promptInstallation();
    expect(first.prompt).toHaveBeenCalledOnce();
    expect(useInstallation.getState()).toMatchObject({
      installed: false,
      accepted: false,
      failed: false,
    });
    const next = offer(browser);
    await promptInstallation();
    expect(next.prompt).toHaveBeenCalledOnce();
  });

  it("only appinstalled confirms installation and clears the offer", async () => {
    const browser = track();
    const event = offer(browser);
    browser.dispatchEvent(new Event("appinstalled"));
    await promptInstallation();
    expect(event.prompt).not.toHaveBeenCalled();
    expect(useInstallation.getState()).toMatchObject({
      installed: true,
      available: false,
    });
  });

  it("does not overwrite appinstalled with a late rejected prompt", async () => {
    const browser = track();
    let reject!: (error: Error) => void;
    const event = offer(browser);
    event.prompt.mockReturnValue(
      new Promise((_, no) => {
        reject = no;
      }),
    );
    const pending = promptInstallation();
    browser.dispatchEvent(new Event("appinstalled"));
    reject(new Error("dialog went away"));
    await pending;
    expect(useInstallation.getState()).toMatchObject({
      installed: true,
      failed: false,
      pending: false,
    });
  });

  it("blocks double clicks while the native prompt is pending", async () => {
    const browser = track();
    let resolve!: () => void;
    const event = offer(browser);
    event.prompt.mockReturnValue(
      new Promise<void>((yes) => {
        resolve = yes;
      }),
    );
    const pending = promptInstallation();
    await promptInstallation();
    expect(event.prompt).toHaveBeenCalledOnce();
    expect(useInstallation.getState().pending).toBe(true);
    resolve();
    await pending;
  });

  it("reports prompt errors and accepts a fresh event for retry", async () => {
    const browser = track();
    offer(browser).prompt.mockRejectedValue(new Error("unsupported"));
    await promptInstallation();
    expect(useInstallation.getState()).toMatchObject({
      failed: true,
      pending: false,
      installed: false,
    });
    offer(browser);
    expect(useInstallation.getState()).toMatchObject({
      failed: false,
      available: true,
    });
  });

  it("recognizes iPhone and iPad desktop-mode browsers", () => {
    const browser = new Browser();
    browser.navigator.userAgent = "iPhone";
    track(browser);
    expect(useInstallation.getState().ios).toBe(true);
    cleanup?.();
    browser.navigator.userAgent = "Macintosh";
    browser.navigator.platform = "MacIntel";
    browser.navigator.maxTouchPoints = 5;
    track(browser);
    expect(useInstallation.getState().ios).toBe(true);
    cleanup?.();
    browser.navigator.maxTouchPoints = 0;
    track(browser);
    expect(useInstallation.getState().ios).toBe(false);
  });

  it("recognizes installed iOS apps and a later standalone display change", () => {
    const browser = new Browser();
    browser.navigator.standalone = true;
    track(browser);
    expect(useInstallation.getState()).toMatchObject({
      installed: true,
      standalone: true,
    });
    expect(offer(browser).defaultPrevented).toBe(false);
    cleanup?.();
    browser.navigator.standalone = false;
    track(browser);
    offer(browser);
    expect(useInstallation.getState().standalone).toBe(false);
    browser.display.matches = true;
    browser.display.dispatchEvent(new Event("change"));
    expect(useInstallation.getState()).toMatchObject({
      installed: true,
      standalone: true,
      available: false,
    });
  });

  it("tells phones from desktop browsers", () => {
    const browser = new Browser();
    track(browser);
    expect(useInstallation.getState().mobile).toBe(true);
    cleanup?.();
    browser.navigator.userAgent = "iPhone";
    track(browser);
    expect(useInstallation.getState().mobile).toBe(true);
    cleanup?.();
    browser.navigator.userAgent = "Mozilla/5.0 (X11; Linux x86_64) Firefox";
    track(browser);
    expect(useInstallation.getState().mobile).toBe(false);
  });

  it("advertises installation on the login page only where it helps", () => {
    const hint = () => installHintUseful(useInstallation.getState());
    // Phones: always, including the HTTPS notice on an insecure address.
    const phone = track();
    expect(hint()).toBe(true);
    cleanup?.();
    phone.isSecureContext = false;
    track(phone);
    expect(hint()).toBe(true);
    cleanup?.();

    // Desktop browsers: only while the browser itself offers a prompt.
    const desktop = new Browser();
    desktop.navigator.userAgent = "Mozilla/5.0 (X11; Linux x86_64) Chrome";
    track(desktop);
    expect(hint()).toBe(false);
    offer(desktop);
    expect(hint()).toBe(true);
    // Just installed from this tab: keep the confirmation visible.
    desktop.dispatchEvent(new Event("appinstalled"));
    expect(useInstallation.getState()).toMatchObject({
      installed: true,
      standalone: false,
    });
    expect(hint()).toBe(true);
    cleanup?.();
    desktop.isSecureContext = false;
    track(desktop);
    expect(hint()).toBe(false);
    cleanup?.();

    // Inside the installed app or the desktop app there is nothing to offer.
    const app = new Browser();
    app.display.matches = true;
    track(app);
    expect(hint()).toBe(false);
    cleanup?.();
    track(new Browser(), true);
    expect(hint()).toBe(false);
  });

  it("does not offer installation over insecure HTTP or in the native desktop app", () => {
    const browser = new Browser();
    browser.isSecureContext = false;
    track(browser);
    expect(offer(browser).defaultPrevented).toBe(false);
    expect(useInstallation.getState().available).toBe(false);
    cleanup?.();
    browser.isSecureContext = true;
    track(browser, true);
    expect(offer(browser).defaultPrevented).toBe(false);
    expect(useInstallation.getState()).toMatchObject({
      available: false,
      native: true,
    });
  });

  it("removes listeners and ignores late results after disposal", async () => {
    const browser = track();
    const event = offer(browser);
    let resolve!: () => void;
    event.prompt.mockReturnValue(
      new Promise<void>((yes) => {
        resolve = yes;
      }),
    );
    const pending = promptInstallation();
    cleanup?.();
    track();
    resolve();
    await pending;
    expect(useInstallation.getState()).toMatchObject({
      accepted: false,
      pending: false,
      installed: false,
    });
    expect(offer(browser).defaultPrevented).toBe(false);
  });
});
