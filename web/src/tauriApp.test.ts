import { afterEach, describe, expect, it, vi } from "vitest";

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
  // Every Tauri app is 0.8.x or older: 0.9 replaced it with the native app.
  it("tells every Tauri app it is outdated", async () => {
    app(async () => ({ version: "0.8.1", features: [] }));
    const { toasts, tauri } = await load();
    tauri.warnIfTauriOutdated();
    const list = toasts.useToasts.getState().toasts;
    expect(list).toHaveLength(1);
    expect(list[0]?.message).toBe(`Die Desktop-App ist veraltet. ${ADVICE}`);
  });

  it("stays quiet in a browser", async () => {
    const { toasts, tauri } = await load();
    tauri.warnIfTauriOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });

  it("warns once per page", async () => {
    app(async () => ({ version: "0.7.0", features: [] }));
    const { toasts, tauri } = await load();
    tauri.warnIfTauriOutdated();
    toasts.useToasts.setState({ toasts: [] });
    tauri.warnIfTauriOutdated();
    expect(toasts.useToasts.getState().toasts).toHaveLength(0);
  });
});
