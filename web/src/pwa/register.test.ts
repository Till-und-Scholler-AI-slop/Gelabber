import { afterEach, describe, expect, it, vi } from "vitest";
import { registerServiceWorker } from "./register.ts";

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
