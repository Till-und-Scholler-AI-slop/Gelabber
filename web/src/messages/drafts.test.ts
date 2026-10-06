import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { bindDraftAccount, loadDraft, saveDraft } from "./drafts.ts";

let stored: Record<string, string>;
beforeEach(() => {
  stored = {};
  const storage = new Proxy(
    {
      getItem: (key: string) => stored[key] ?? null,
      setItem: (key: string, value: string) => {
        stored[key] = value;
      },
      removeItem: (key: string) => {
        delete stored[key];
      },
    },
    {
      ownKeys: () => Object.keys(stored),
      getOwnPropertyDescriptor: () => ({
        enumerable: true,
        configurable: true,
      }),
    },
  );
  vi.stubGlobal("sessionStorage", storage);
  bindDraftAccount(null);
});
afterEach(() => {
  bindDraftAccount(null);
  vi.unstubAllGlobals();
});

describe("account and chat scoped drafts", () => {
  it("keeps text and the original file across navigation, without serializing the file", () => {
    bindDraftAccount("a");
    const file = new File(["original bytes"], "report.txt", {
      type: "text/plain",
    });
    saveDraft("a", "one", { text: "draft", file });
    saveDraft("a", "two", { text: "another", file: null });
    expect(loadDraft("a", "one")).toEqual({ text: "draft", file });
    expect(loadDraft("a", "two").text).toBe("another");
    expect(Object.values(stored).sort()).toEqual(["another", "draft"]);
  });
  it("restores a reload text draft only for the session's account", () => {
    stored["gelabber:chat-draft:a:reload"] = "restored";
    stored["gelabber:chat-draft:b:reload"] = "foreign";
    bindDraftAccount("a");
    expect(loadDraft("a", "reload")).toEqual({ text: "restored", file: null });
    expect(loadDraft("b", "reload").text).toBe("");
  });
  it("releases files and storage on logout and on a different account", () => {
    bindDraftAccount("a");
    saveDraft("a", "one", { text: "private", file: new File(["a"], "a.txt") });
    bindDraftAccount("b");
    expect(loadDraft("a", "one")).toEqual({ text: "", file: null });
    saveDraft("b", "one", { text: "next", file: null });
    bindDraftAccount(null);
    expect(loadDraft("b", "one")).toEqual({ text: "", file: null });
    expect(Object.keys(stored)).toEqual([]);
  });
  it("owns image preview URLs through navigation and releases them on replacement/logout", () => {
    bindDraftAccount("a");
    const create = vi
      .spyOn(URL, "createObjectURL")
      .mockReturnValue("blob:draft");
    const revoke = vi
      .spyOn(URL, "revokeObjectURL")
      .mockImplementation(() => {});
    const file = new File(["image"], "image.png", { type: "image/png" });
    saveDraft("a", "image", { text: "caption", file });
    loadDraft("a", "image");
    saveDraft("a", "image", { text: "changed caption", file });
    expect(create).toHaveBeenCalledTimes(1);
    expect(revoke).not.toHaveBeenCalled();
    bindDraftAccount(null);
    expect(revoke).toHaveBeenCalledWith("blob:draft");
    create.mockRestore();
    revoke.mockRestore();
  });
  it("clears the submitted draft without dropping another channel", () => {
    bindDraftAccount("a");
    saveDraft("a", "one", { text: "send", file: null });
    saveDraft("a", "two", { text: "keep", file: null });
    saveDraft("a", "one", { text: "", file: null });
    expect(loadDraft("a", "one").text).toBe("");
    expect(loadDraft("a", "two").text).toBe("keep");
  });
  it("preserves memory drafts when browser storage is unavailable", () => {
    vi.stubGlobal("sessionStorage", {
      getItem: () => {
        throw new Error("disabled");
      },
      setItem: () => {
        throw new Error("quota");
      },
      removeItem: () => {
        throw new Error("disabled");
      },
    });
    saveDraft("a", "one", { text: "still here", file: null });
    expect(loadDraft("a", "one").text).toBe("still here");
  });
});
