import { afterEach, describe, expect, it, vi } from "vitest";

import { NOTIFICATION_OPEN } from "./notifications.ts";
import { startPwa } from "./start.ts";

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

/** A browser whose page has loaded, and a worker that reports a tap. */
function browser() {
  const register = vi.fn().mockResolvedValue({});
  const serviceWorker = Object.assign(new EventTarget(), { register });
  vi.stubGlobal(
    "window",
    Object.assign(new EventTarget(), { isSecureContext: true }),
  );
  vi.stubGlobal("document", { readyState: "complete" });
  vi.stubGlobal("navigator", { serviceWorker });
  const navigate = vi.fn();
  const tap = () =>
    serviceWorker.dispatchEvent(
      new MessageEvent("message", {
        data: {
          type: NOTIFICATION_OPEN,
          path: "/s/srv/c/chan",
          user: "user-1",
        },
      }),
    );
  return { register, navigate, tap };
}

describe("what the production build starts in a browser", () => {
  it("registers the worker and sends a notification tap through the router", () => {
    vi.stubEnv("PROD", true);
    const { register, navigate, tap } = browser();
    expect(startPwa({ desktop: false, user: () => "user-1", navigate })).toBe(
      true,
    );
    expect(register).toHaveBeenCalledExactlyOnceWith("/sw.js", {
      scope: "/",
      updateViaCache: "none",
    });
    tap();
    expect(navigate.mock.calls).toEqual([[{ href: "/s/srv/c/chan" }]]);
  });

  it("starts neither under Vite's dev server", () => {
    vi.stubEnv("PROD", false);
    const { register, navigate, tap } = browser();
    expect(startPwa({ desktop: false, user: () => "user-1", navigate })).toBe(
      false,
    );
    tap();
    expect(register).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });

  it("starts neither in the desktop app", () => {
    vi.stubEnv("PROD", true);
    const { register, navigate, tap } = browser();
    expect(startPwa({ desktop: true, user: () => "user-1", navigate })).toBe(
      false,
    );
    tap();
    expect(register).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });
});
