import { afterEach, describe, expect, it } from "vitest";

import { clearEditDrafts, setEditDraft, useEditDrafts } from "./editDrafts.ts";

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
});
