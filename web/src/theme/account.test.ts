import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { api, ApiError } from "../api/client.ts";
import {
  bindThemeAccount,
  changeTheme,
  retryThemes,
  syncThemes,
  useAccountThemes,
} from "./account.ts";
import { importTheme, type ThemeDocument } from "./model.ts";
import { BUILTIN_THEMES } from "./presets.ts";
vi.mock("../api/client.ts", () => ({
  api: vi.fn(),
  ApiError: class extends Error {
    constructor(
      _code: string,
      public status: number,
      message: string,
    ) {
      super(message);
    }
  },
}));
const call = vi.mocked(api);
const doc = (active = "dark", revision = 1): ThemeDocument => ({
  version: 1,
  revision,
  active,
  customThemes: [],
});
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}
beforeEach(() => {
  bindThemeAccount(null);
  call.mockReset();
  const data = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => data.set(key, value),
    removeItem: (key: string) => data.delete(key),
  });
});
afterEach(() => {
  bindThemeAccount(null);
  vi.unstubAllGlobals();
});
describe("account theme sync", () => {
  it("migrates a legacy mode only into an uninitialised account", async () => {
    localStorage.setItem("gelabber.theme", "light");
    bindThemeAccount("a");
    call
      .mockResolvedValueOnce(doc("dark", 0))
      .mockResolvedValueOnce(doc("light", 1));
    await syncThemes();
    expect(call).toHaveBeenLastCalledWith("/me/themes", {
      method: "PUT",
      body: doc("light", 0),
    });
    expect(useAccountThemes.getState().doc.active).toBe("light");
  });
  it("ignores late responses from the previous account, including a save", async () => {
    bindThemeAccount("a");
    call.mockResolvedValueOnce(doc());
    await syncThemes();
    const pending = deferred<ThemeDocument>();
    call.mockReturnValueOnce(pending.promise);
    const save = changeTheme({ kind: "apply", id: "nord" });
    bindThemeAccount("b");
    call.mockResolvedValueOnce(doc("gruvbox"));
    await syncThemes();
    pending.resolve(doc("nord", 2));
    await save;
    expect(useAccountThemes.getState()).toMatchObject({
      owner: "b",
      doc: { active: "gruvbox" },
      pending: null,
    });
  });
  it("keeps offline changes across reloads and rebases retry without losing other themes", async () => {
    bindThemeAccount("a");
    call.mockResolvedValueOnce(doc());
    await syncThemes();
    call.mockRejectedValueOnce(new Error("offline"));
    await changeTheme({ kind: "apply", id: "nord" });
    bindThemeAccount(null);
    bindThemeAccount("a");
    expect(useAccountThemes.getState()).toMatchObject({
      pending: { kind: "apply", id: "nord" },
      status: "error",
    });
    const theme = importTheme(JSON.stringify(BUILTIN_THEMES[2]));
    const remote = { ...doc("gruvbox", 4), customThemes: [theme] };
    call
      .mockResolvedValueOnce(remote)
      .mockResolvedValueOnce({ ...remote, active: "nord", revision: 5 });
    await retryThemes();
    expect(call).toHaveBeenLastCalledWith("/me/themes", {
      method: "PUT",
      body: { ...remote, active: "nord" },
    });
    expect(useAccountThemes.getState()).toMatchObject({
      pending: null,
      doc: { revision: 5, customThemes: [theme] },
    });
  });
  it("retains the draft operation on revision conflict and blocks destructive replacement", async () => {
    bindThemeAccount("a");
    call.mockResolvedValueOnce(doc());
    await syncThemes();
    call.mockRejectedValueOnce(new ApiError("theme_conflict", 409, "conflict"));
    await changeTheme({ kind: "apply", id: "nord" });
    expect(useAccountThemes.getState().status).toBe("conflict");
    expect(await changeTheme({ kind: "apply", id: "light" })).toBe(false);
    expect(useAccountThemes.getState().pending).toEqual({
      kind: "apply",
      id: "nord",
    });
  });
  it("deletes an active custom theme and falls back to dark", async () => {
    const theme = importTheme(JSON.stringify(BUILTIN_THEMES[0]));
    bindThemeAccount("a");
    call.mockResolvedValueOnce({ ...doc(theme.id), customThemes: [theme] });
    await syncThemes();
    call.mockResolvedValueOnce(doc("dark", 2));
    await changeTheme({ kind: "delete", id: theme.id });
    expect(call).toHaveBeenLastCalledWith("/me/themes", {
      method: "PUT",
      body: doc("dark", 1),
    });
  });
  it("discards corrupt cache and never overwrites an existing account during migration", async () => {
    localStorage.setItem("gelabber.themes.v1:a", "{broken");
    localStorage.setItem("gelabber.theme", "light");
    bindThemeAccount("a");
    call.mockResolvedValueOnce(doc("nord", 12));
    await syncThemes();
    expect(call).toHaveBeenCalledTimes(1);
    expect(useAccountThemes.getState().doc.active).toBe("nord");
  });
});
