import { describe, expect, it, vi } from "vitest";

import { keepScreenAwake } from "./wakeLock.ts";

class Sentinel extends EventTarget {
  release = vi.fn(async () => {
    this.dispatchEvent(new Event("release"));
  });
}

class Page extends EventTarget {
  visibilityState = "visible";
  show(visible: boolean) {
    this.visibilityState = visible ? "visible" : "hidden";
    this.dispatchEvent(new Event("visibilitychange"));
  }
}

function browser({ coarse = true, supported = true } = {}) {
  const sentinels: Sentinel[] = [];
  const request = vi.fn(async (type: string) => {
    void type;
    const sentinel = new Sentinel();
    sentinels.push(sentinel);
    return sentinel;
  });
  const document = new Page();
  const target = {
    document,
    navigator: supported ? { wakeLock: { request } } : {},
    matchMedia: (query: string) => ({
      matches: query === "(pointer: coarse)" && coarse,
    }),
  } as unknown as Window;
  return { target, document, request, sentinels };
}
const settled = () => new Promise((resolve) => setTimeout(resolve));

describe("screen wake lock", () => {
  it("holds a screen lock until stopped", async () => {
    const { target, request, sentinels } = browser();
    const stop = keepScreenAwake(target);
    await settled();
    expect(request).toHaveBeenCalledExactlyOnceWith("screen");
    expect(sentinels[0]?.release).not.toHaveBeenCalled();
    stop();
    expect(sentinels[0]?.release).toHaveBeenCalledOnce();
  });

  it("stays out of the way without the API or without a touch screen", async () => {
    const missing = browser({ supported: false });
    expect(() => keepScreenAwake(missing.target)()).not.toThrow();
    const mouse = browser({ coarse: false });
    keepScreenAwake(mouse.target)();
    await settled();
    expect(mouse.request).not.toHaveBeenCalled();
  });

  it("requests again after the browser dropped the lock in the background", async () => {
    const { target, document, request, sentinels } = browser();
    const stop = keepScreenAwake(target);
    await settled();
    // A second event while the lock is held must not stack requests.
    document.show(true);
    expect(request).toHaveBeenCalledOnce();

    document.show(false);
    sentinels[0]?.dispatchEvent(new Event("release"));
    expect(request).toHaveBeenCalledOnce();
    document.show(true);
    await settled();
    expect(request).toHaveBeenCalledTimes(2);

    stop();
    expect(sentinels[0]?.release).not.toHaveBeenCalled();
    expect(sentinels[1]?.release).toHaveBeenCalledOnce();
    document.show(true);
    expect(request).toHaveBeenCalledTimes(2);
  });

  it("waits for a visible page before asking", async () => {
    const { target, document, request } = browser();
    document.visibilityState = "hidden";
    const stop = keepScreenAwake(target);
    expect(request).not.toHaveBeenCalled();
    document.show(true);
    expect(request).toHaveBeenCalledOnce();
    stop();
  });

  it("swallows a refusal and tries again on the next return", async () => {
    const { target, document, request } = browser();
    request.mockRejectedValueOnce(
      new DOMException("denied", "NotAllowedError"),
    );
    const stop = keepScreenAwake(target);
    await settled();
    expect(request).toHaveBeenCalledOnce();
    document.show(true);
    await settled();
    expect(request).toHaveBeenCalledTimes(2);
    stop();
  });

  it("releases a lock that arrives after leaving the call", async () => {
    const { target, sentinels } = browser();
    keepScreenAwake(target)();
    await settled();
    expect(sentinels[0]?.release).toHaveBeenCalledOnce();
  });
});
