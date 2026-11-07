import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_VERSION } from "./version.ts";

// The warning is once per page, so each test loads a fresh set of modules.
async function load() {
  vi.resetModules();
  const toasts = await import("./components/toasts.ts");
  const tauri = await import("./tauriApp.ts");
  return { toasts, tauri };
}

function app(media_info: () => Promise<unknown>) {
  vi.stubGlobal("window", {
    __TAURI_INTERNALS__: {
      invoke: async (command: string) => {
        if (command !== "media_info") throw new Error(`unexpected ${command}`);
        return media_info();
      },
    },
  });
}

const ADVICE =
  "Ab 0.9 gibt es eine neue Desktop-App: unter Windows ab v0.7 über das App-Update, " +
  "unter Arch mit pacman -Syu, sonst gelabber-native-* aus dem GitHub-Release.";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("olderVersion", () => {
  it("compares the dotted parts as numbers", async () => {
    const { tauri } = await load();
    expect(tauri.olderVersion("0.7.0", "0.8.1")).toBe(true);
    expect(tauri.olderVersion("0.8.1", "0.8.1")).toBe(false);
    expect(tauri.olderVersion("0.9.0", "0.8.1")).toBe(false);
    expect(tauri.olderVersion("0.10.0", "0.9.9")).toBe(false);
    expect(tauri.olderVersion("0.8", "0.8.1")).toBe(true);
    expect(tauri.olderVersion("0.8.0", "0.8")).toBe(false);
  });
});

describe("markTauriRoot", () => {
  const fakeRoot = () => ({ dataset: {} }) as unknown as HTMLElement;

  it("marks the root in the Tauri app", async () => {
    app(async () => ({}));
    const { tauri } = await load();
    const root = fakeRoot();
    tauri.markTauriRoot(root);
    expect(root.dataset.tauri).toBe("");
  });

  it("leaves a browser's root alone", async () => {
    vi.stubGlobal("window", { __TAURI_INTERNALS__: null });
    const { tauri } = await load();
    const root = fakeRoot();
    tauri.markTauriRoot(root);
    expect(root.dataset.tauri).toBeUndefined();
  });
});

describe("warnIfTauriOutdated", () => {
  it("names both versions for an app older than the web client", async () => {
    app(async () => ({ version: "0.7.0", features: [] }));
    const { toasts, tauri } = await load();
    await tauri.warnIfTauriOutdated();
    const list = toasts.useToasts.getState().toasts;
    expect(list).toHaveLength(1);
    expect(list[0]?.message).toBe(
      `Die Desktop-App ist veraltet (v0.7.0, Server v${APP_VERSION}). ${ADVICE}`,
    );
  });

  it("stays quiet for an app as new as the web client", async () => {
    app(async () => ({ version: APP_VERSION, features: [] }));
    const { toasts, tauri } = await load();
    await tauri.warnIfTauriOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });

  it("stays quiet in a browser", async () => {
    const { toasts, tauri } = await load();
    await tauri.warnIfTauriOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });

  it("warns without versions for an app that cannot tell its own", async () => {
    app(async () => {
      throw new Error("unknown command");
    });
    const { toasts, tauri } = await load();
    await tauri.warnIfTauriOutdated();
    const list = toasts.useToasts.getState().toasts;
    expect(list).toHaveLength(1);
    expect(list[0]?.message).toBe(`Die Desktop-App ist veraltet. ${ADVICE}`);
  });

  it("warns once per page", async () => {
    app(async () => ({ version: "0.7.0", features: [] }));
    const { toasts, tauri } = await load();
    await tauri.warnIfTauriOutdated();
    toasts.useToasts.setState({ toasts: [] });
    await tauri.warnIfTauriOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });
});
