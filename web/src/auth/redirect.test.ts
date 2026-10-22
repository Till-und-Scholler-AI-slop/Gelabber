import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  PENDING_REDIRECT_MS,
  isInviteRedirect,
  safeRedirect,
  savePendingRedirect,
  takePendingRedirect,
} from "./redirect.ts";

function memoryStorage() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}

describe("safe redirect", () => {
  it("accepts paths of this app", () => {
    expect(safeRedirect("/invite/abc")).toBe("/invite/abc");
    expect(safeRedirect("/s/1/c/2?x=1")).toBe("/s/1/c/2?x=1");
  });

  it("rejects targets that leave the app", () => {
    expect(safeRedirect("https://evil.example/")).toBeUndefined();
    expect(safeRedirect("//evil.example/")).toBeUndefined();
    expect(safeRedirect("/\\evil.example/")).toBeUndefined();
    expect(safeRedirect("invite/abc")).toBeUndefined();
    expect(safeRedirect(42)).toBeUndefined();
  });

  it("recognises an invite target", () => {
    expect(isInviteRedirect("/invite/abc")).toBe(true);
    expect(isInviteRedirect("/invite/")).toBe(false);
    expect(isInviteRedirect("/s/1")).toBe(false);
    expect(isInviteRedirect(undefined)).toBe(false);
  });
});

describe("pending redirect", () => {
  let storage: ReturnType<typeof memoryStorage>;
  beforeEach(() => {
    storage = memoryStorage();
    vi.stubGlobal("localStorage", storage);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("carries the invite from register to the confirmation link, once", () => {
    // RegisterPage: the server mailed a link, nobody is signed in.
    savePendingRedirect("/invite/abc", 1_000);
    // VerifyEmailPage, later, in the tab the mail opened.
    expect(takePendingRedirect(2_000)).toBe("/invite/abc");
    expect(takePendingRedirect(2_000)).toBeUndefined();
  });

  it("expires after a day", () => {
    savePendingRedirect("/invite/abc", 0);
    expect(takePendingRedirect(PENDING_REDIRECT_MS)).toBeUndefined();
    savePendingRedirect("/invite/abc", 0);
    expect(takePendingRedirect(PENDING_REDIRECT_MS - 1)).toBe("/invite/abc");
  });

  it("never stores or returns a target outside the app", () => {
    savePendingRedirect("//evil.example/");
    expect(storage.getItem("gelabber.pending-redirect")).toBeNull();
    storage.setItem(
      "gelabber.pending-redirect",
      JSON.stringify({ target: "https://evil.example/", expires: 9e15 }),
    );
    expect(takePendingRedirect()).toBeUndefined();
    storage.setItem("gelabber.pending-redirect", "{not json");
    expect(takePendingRedirect()).toBeUndefined();
  });

  it("does without storage", () => {
    vi.stubGlobal("localStorage", {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("blocked");
      },
      removeItem: () => {},
    });
    expect(() => savePendingRedirect("/invite/abc")).not.toThrow();
    expect(takePendingRedirect()).toBeUndefined();
  });
});
