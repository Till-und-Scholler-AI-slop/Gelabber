import { MutationObserver, QueryClient } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";

import { nextScopeGeneration } from "../auth/scope.ts";

import {
  clearEditDrafts,
  resetEditDrafts,
  saveEdit,
  setEditDraft,
  useEditDrafts,
} from "./editDrafts.ts";

const draftOf = (id: string) => useEditDrafts.getState().byMessage[id]?.text;

describe("edit drafts", () => {
  afterEach(() => useEditDrafts.setState({ byMessage: {} }));

  // A row scrolled out of the virtual list unmounts; the edit must not go
  // with it.
  it("keeps an open edit apart from the row", () => {
    setEditDraft("c1", "m1", "halb geschrieben");
    expect(draftOf("m1")).toBe("halb geschrieben");
    setEditDraft("c1", "m1", "halb geschrieben, jetzt fertig");
    expect(draftOf("m1")).toBe("halb geschrieben, jetzt fertig");
  });

  it("closes an edit", () => {
    setEditDraft("c1", "m1", "text");
    setEditDraft("c1", "m1", null);
    expect(draftOf("m1")).toBeUndefined();
  });

  it("drops only the closed conversation's edits", () => {
    setEditDraft("c1", "m1", "eins");
    setEditDraft("c2", "m2", "zwei");
    clearEditDrafts("c1");
    expect(draftOf("m1")).toBeUndefined();
    expect(draftOf("m2")).toBe("zwei");
  });

  // Account switch: what one account typed must not open for the next.
  it("drops every open edit on reset", () => {
    setEditDraft("c1", "m1", "eins");
    setEditDraft("c2", "m2", "zwei");
    resetEditDrafts();
    expect(useEditDrafts.getState().byMessage).toEqual({});
  });
});

describe("saving edits", () => {
  // One edit mutation serves every row; TanStack hands a call's own
  // onError only to the latest call.
  it("tells each row about its own failed save", async () => {
    let rejectFirst: (error: Error) => void = () => {};
    const first = new Promise((_, reject) => (rejectFirst = reject));
    const observer = new MutationObserver(new QueryClient(), {
      mutationFn: ({ id }: { id: string; content: string }) =>
        id === "a" ? first : Promise.resolve(),
    });
    const edit = { mutateAsync: observer.mutate.bind(observer) };
    const failedA = vi.fn();
    const failedB = vi.fn();
    saveEdit(edit, "a")("neu a", failedA);
    saveEdit(edit, "b")("neu b", failedB);
    rejectFirst(new Error("abgelehnt"));
    await vi.waitFor(() => expect(failedA).toHaveBeenCalledOnce());
    expect(failedB).not.toHaveBeenCalled();
  });

  // The account changed while the save was in flight: reopening the editor
  // would hand the old account's text to the next one.
  it("does not reopen an edit that failed after an account switch", async () => {
    let reject: (error: Error) => void = () => {};
    const pending = new Promise((_, rejectWith) => (reject = rejectWith));
    const failed = vi.fn();
    const settled = vi.fn();
    saveEdit({ mutateAsync: () => pending.finally(settled) }, "a")(
      "geheim",
      failed,
    );
    nextScopeGeneration();
    reject(new Error("abgelehnt"));
    await vi.waitFor(() => expect(settled).toHaveBeenCalledOnce());
    await Promise.resolve();
    expect(failed).not.toHaveBeenCalled();
  });
});
