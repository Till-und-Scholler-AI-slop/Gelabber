import { afterEach, describe, expect, it, vi } from "vitest";
import { activeServiceWorker, registerServiceWorker } from "./register.ts";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

function browser(readyState = "complete") {
  const window = Object.assign(new EventTarget(), { isSecureContext: true });
  const register = vi.fn().mockResolvedValue({});
  vi.stubGlobal("window", window);
  vi.stubGlobal("document", { readyState });
  vi.stubGlobal("navigator", { serviceWorker: { register } });
  return { window, register };
}

describe("service worker registration", () => {
  it("registers a fresh root-scoped worker after loading", () => {
    const { window, register } = browser("loading");
    registerServiceWorker();
    expect(register).not.toHaveBeenCalled();
    window.dispatchEvent(new Event("load"));
    window.dispatchEvent(new Event("load"));
    expect(register).toHaveBeenCalledExactlyOnceWith("/sw.js", {
      scope: "/",
      updateViaCache: "none",
    });
  });

  it("also registers when the document has already finished loading", () => {
    const { register } = browser();
    registerServiceWorker();
    expect(register).toHaveBeenCalledOnce();
  });

  it("does not register over insecure HTTP or without browser support", () => {
    const { window, register } = browser();
    window.isSecureContext = false;
    registerServiceWorker();
    expect(register).not.toHaveBeenCalled();
    window.isSecureContext = true;
    vi.stubGlobal("navigator", {});
    expect(registerServiceWorker).not.toThrow();
  });

  it("contains registration failure without an unhandled rejection", async () => {
    const { register } = browser();
    register.mockRejectedValue(new Error("offline"));
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    registerServiceWorker();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(warn).toHaveBeenCalledOnce();
  });
});

describe("the worker this page registered", () => {
  it("is not looked up where none was registered (vite dev, desktop app)", async () => {
    vi.resetModules();
    const fresh = await import("./register.ts");
    const getRegistration = vi.fn();
    vi.stubGlobal("navigator", { serviceWorker: { getRegistration } });
    expect(await fresh.activeServiceWorker()).toBeUndefined();
    // Insecure HTTP never registers either.
    vi.stubGlobal("window", { isSecureContext: false });
    fresh.registerServiceWorker();
    expect(await fresh.activeServiceWorker()).toBeUndefined();
    expect(getRegistration).not.toHaveBeenCalled();
  });

  it("is available once active, and not before or when the lookup fails", async () => {
    const { register } = browser();
    registerServiceWorker();
    expect(register).toHaveBeenCalledOnce();
    const getRegistration = vi.fn();
    vi.stubGlobal("navigator", { serviceWorker: { getRegistration } });
    const active = { active: {} };
    getRegistration.mockResolvedValueOnce(active);
    expect(await activeServiceWorker()).toBe(active);
    getRegistration.mockResolvedValueOnce({ active: null, installing: {} });
    expect(await activeServiceWorker()).toBeUndefined();
    getRegistration.mockResolvedValueOnce(undefined);
    expect(await activeServiceWorker()).toBeUndefined();
    getRegistration.mockRejectedValueOnce(new Error("SecurityError"));
    expect(await activeServiceWorker()).toBeUndefined();
  });
});
