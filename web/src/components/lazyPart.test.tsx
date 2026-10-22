import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { lazyPart, preloadPartsWhenIdle } from "./lazyPart.tsx";
import { LazyPartFailedView } from "./LazyPartFailed.tsx";

function Hello({ name }: { name: string }) {
  return <p>Hallo {name}</p>;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("lazy parts", () => {
  it("shows the fallback until loaded, then the part at once", async () => {
    const load = vi.fn(() => Promise.resolve(Hello));
    const part = lazyPart(load, "Fehlt.");
    expect(load).not.toHaveBeenCalled();
    expect(
      renderToStaticMarkup(
        <part.Component name="Ada" fallback={<span>lädt</span>} />,
      ),
    ).toBe("<span>lädt</span>");
    await part.preload();
    expect(part.loaded()).toBe(true);
    expect(
      renderToStaticMarkup(
        <part.Component name="Ada" fallback={<span>lädt</span>} />,
      ),
    ).toBe("<p>Hallo Ada</p>");
    await part.preload();
    expect(load).toHaveBeenCalledTimes(1);
  });

  it("keeps a failed load, as the browser does", async () => {
    const load = vi
      .fn<() => Promise<typeof Hello>>()
      .mockRejectedValue(new TypeError("Failed to fetch module"));
    const part = lazyPart(load, "Fehlt.");
    await expect(part.preload()).resolves.toBeUndefined();
    await part.preload();
    expect(part.loaded()).toBe(false);
    expect(load).toHaveBeenCalledTimes(1);
    expect(
      renderToStaticMarkup(<part.Component name="Ada" fallback="lädt" />),
    ).toBe("lädt");
  });

  it("preloads the parts not loaded yet once the browser is idle", async () => {
    const idle: (() => void)[] = [];
    vi.stubGlobal("window", {
      requestIdleCallback: (run: () => void) => idle.push(run),
      cancelIdleCallback: vi.fn(),
    });
    const load = vi.fn(() => Promise.resolve(Hello));
    const part = lazyPart(load, "Fehlt.");
    const stop = preloadPartsWhenIdle();
    expect(load).not.toHaveBeenCalled();
    for (const run of idle) run();
    await vi.waitFor(() => expect(part.loaded()).toBe(true));
    stop();
  });

  it("falls back to a timer without requestIdleCallback", () => {
    const timers: number[] = [];
    vi.stubGlobal("window", {
      setTimeout: (_run: () => void, ms: number) => timers.push(ms),
      clearTimeout: vi.fn(),
    });
    preloadPartsWhenIdle()();
    expect(timers).toEqual([2_000]);
  });
});

describe("a part that failed to load", () => {
  it("offers a page reload outside a call", () => {
    const html = renderToStaticMarkup(
      <LazyPartFailedView failure="Fehlt." inCall={false} />,
    );
    expect(html).toContain('role="alert"');
    expect(html).toContain("Fehlt.");
    expect(html).toContain("Seite neu laden");
  });

  it("never offers a reload during a call, which would end it", () => {
    const html = renderToStaticMarkup(
      <LazyPartFailedView failure="Fehlt." inCall />,
    );
    expect(html).toContain("Fehlt.");
    expect(html).not.toContain("Seite neu laden");
    expect(html).toContain("nach dem Anruf neu");
  });
});
