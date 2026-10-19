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
    saveDraft("a", "one", { text: "draft", files: [file] });
    saveDraft("a", "two", { text: "another", files: [] });
    expect(loadDraft("a", "one")).toEqual({ text: "draft", files: [file] });
    expect(loadDraft("a", "two").text).toBe("another");
    expect(Object.values(stored).sort()).toEqual(["another", "draft"]);
  });
  it("restores a reload text draft only for the session's account", () => {
    stored["gelabber:chat-draft:a:reload"] = "restored";
    stored["gelabber:chat-draft:b:reload"] = "foreign";
    bindDraftAccount("a");
    expect(loadDraft("a", "reload")).toEqual({ text: "restored", files: [] });
    expect(loadDraft("b", "reload").text).toBe("");
  });
  it("releases files and storage on logout and on a different account", () => {
    bindDraftAccount("a");
    saveDraft("a", "one", {
      text: "private",
      files: [new File(["a"], "a.txt")],
    });
    bindDraftAccount("b");
    expect(loadDraft("a", "one")).toEqual({ text: "", files: [] });
    saveDraft("b", "one", { text: "next", files: [] });
    bindDraftAccount(null);
    expect(loadDraft("b", "one")).toEqual({ text: "", files: [] });
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
    saveDraft("a", "image", { text: "caption", files: [file] });
    loadDraft("a", "image");
    saveDraft("a", "image", { text: "changed caption", files: [file] });
    expect(create).toHaveBeenCalledTimes(1);
    expect(revoke).not.toHaveBeenCalled();
    saveDraft("a", "image", {
      text: "two",
      files: [file, new File(["doc"], "doc.pdf", { type: "application/pdf" })],
    });
    expect(create).toHaveBeenCalledTimes(1);
    bindDraftAccount(null);
    expect(revoke).toHaveBeenCalledWith("blob:draft");
    create.mockRestore();
    revoke.mockRestore();
  });
  it("clears the submitted draft without dropping another channel", () => {
    bindDraftAccount("a");
    saveDraft("a", "one", { text: "send", files: [] });
    saveDraft("a", "two", { text: "keep", files: [] });
    saveDraft("a", "one", { text: "", files: [] });
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
    saveDraft("a", "one", { text: "still here", files: [] });
    expect(loadDraft("a", "one").text).toBe("still here");
  });
});
