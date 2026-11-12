import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  EMPTY_DRAFT,
  clearFeedbackDraft,
  closeFeedbackDialog,
  openFeedbackDialog,
  setFeedbackImage,
  updateFeedbackDraft,
  useFeedbackDialog,
} from "./dialogStore.ts";

function reset() {
  clearFeedbackDraft();
  useFeedbackDialog.setState({ open: false });
}

describe("the draft", () => {
  beforeEach(reset);
  afterEach(reset);

  it("stays, image included, when the dialog closes", () => {
    openFeedbackDialog();
    const image = new File(["x"], "a.webp", { type: "image/webp" });
    updateFeedbackDraft({ kind: "problem", text: "Kaputt" });
    setFeedbackImage(image);
    closeFeedbackDialog();
    openFeedbackDialog();
    const { open, draft } = useFeedbackDialog.getState();
    expect(open).toBe(true);
    expect(draft).toMatchObject({ kind: "problem", text: "Kaputt", image });
  });

  it("is gone once sent", () => {
    updateFeedbackDraft({ text: "Danke" });
    setFeedbackImage(new File(["x"], "a.png", { type: "image/png" }));
    clearFeedbackDraft();
    expect(useFeedbackDialog.getState().draft).toEqual(EMPTY_DRAFT);
  });

  it("does not follow into another sign-in", () => {
    updateFeedbackDraft({ text: "Von Lena" });
    useFeedbackDialog.setState({ generation: -1 });
    openFeedbackDialog();
    expect(useFeedbackDialog.getState().draft).toEqual(EMPTY_DRAFT);
  });

  it("lets a second image replace the first", () => {
    const revoke = vi.spyOn(URL, "revokeObjectURL");
    try {
      setFeedbackImage(new File(["1"], "1.png", { type: "image/png" }));
      const first = useFeedbackDialog.getState().draft.preview;
      const second = new File(["2"], "2.png", { type: "image/png" });
      setFeedbackImage(second);
      expect(revoke).toHaveBeenCalledWith(first);
      expect(useFeedbackDialog.getState().draft.image).toBe(second);
    } finally {
      revoke.mockRestore();
    }
  });
});
