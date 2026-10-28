import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { APP_VERSION } from "../../version.ts";

// The warning is once per page, so each test loads a fresh set of modules.
async function load() {
  vi.resetModules();
  const bridge = await import("./bridge.ts");
  const toasts = await import("../../components/toasts.ts");
  const outdated = await import("./outdated.ts");
  return { bridge, toasts, outdated };
}

function app(
  bridge: typeof import("./bridge.ts"),
  media_info: () => Promise<unknown>,
) {
  bridge.setNativeBridgeForTests({
    invoke: async (command: string) => {
      if (command !== "media_info") throw new Error(`unexpected ${command}`);
      return media_info();
    },
    channel: async () => null,
  } as unknown as import("./bridge.ts").NativeBridge);
}

const ADVICE =
  "Aktualisiere sie, damit Hardware-Codecs wie AV1 und H.264 genutzt werden.";

describe("olderVersion", () => {
  it("compares the dotted parts as numbers", async () => {
    const { outdated } = await load();
    expect(outdated.olderVersion("0.7.0", "0.8.1")).toBe(true);
    expect(outdated.olderVersion("0.8.1", "0.8.1")).toBe(false);
    expect(outdated.olderVersion("0.9.0", "0.8.1")).toBe(false);
    expect(outdated.olderVersion("0.10.0", "0.9.9")).toBe(false);
    expect(outdated.olderVersion("0.8", "0.8.1")).toBe(true);
    expect(outdated.olderVersion("0.8.0", "0.8")).toBe(false);
  });
});

describe("warnIfDesktopOutdated", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });
  afterEach(async () => {
    const { bridge } = await load();
    bridge.setNativeBridgeForTests(undefined);
  });

  it("names both versions for an app older than the web client", async () => {
    const { bridge, toasts, outdated } = await load();
    app(bridge, async () => ({ version: "0.7.0", features: [] }));
    await outdated.warnIfDesktopOutdated();
    const list = toasts.useToasts.getState().toasts;
    expect(list).toHaveLength(1);
    expect(list[0]?.message).toBe(
      `Die Desktop-App ist veraltet (v0.7.0, Server v${APP_VERSION}). ${ADVICE}`,
    );
  });

  it("stays quiet for an app as new as the web client", async () => {
    const { bridge, toasts, outdated } = await load();
    app(bridge, async () => ({ version: APP_VERSION, features: [] }));
    await outdated.warnIfDesktopOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });

  it("stays quiet in a browser", async () => {
    const { toasts, outdated } = await load();
    await outdated.warnIfDesktopOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });

  it("warns without versions for an app that cannot tell its own", async () => {
    const { bridge, toasts, outdated } = await load();
    app(bridge, async () => {
      throw new Error("unknown command");
    });
    await outdated.warnIfDesktopOutdated();
    const list = toasts.useToasts.getState().toasts;
    expect(list).toHaveLength(1);
    expect(list[0]?.message).toBe(`Die Desktop-App ist veraltet. ${ADVICE}`);
  });

  it("warns once per page", async () => {
    const { bridge, toasts, outdated } = await load();
    app(bridge, async () => ({ version: "0.7.0", features: [] }));
    await outdated.warnIfDesktopOutdated();
    toasts.useToasts.setState({ toasts: [] });
    await outdated.warnIfDesktopOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });
});
