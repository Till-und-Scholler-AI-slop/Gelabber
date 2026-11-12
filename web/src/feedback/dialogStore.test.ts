import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  EMPTY_DRAFT,
  clearFeedbackDraft,
  closeFeedbackDialog,
  dropStaleFeedbackDialog,
  feedbackDialogOpen,
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

  it("is not open for the next account when a session ends while open", () => {
    openFeedbackDialog();
    updateFeedbackDraft({ text: "Von Lena" });
    // Lena's session ends with the dialog open; Tom signs in.
    useFeedbackDialog.setState({ generation: -1 });
    expect(feedbackDialogOpen(useFeedbackDialog.getState())).toBe(false);
    dropStaleFeedbackDialog();
    const state = useFeedbackDialog.getState();
    expect(state.open).toBe(false);
    expect(state.draft).toEqual(EMPTY_DRAFT);
    openFeedbackDialog();
    expect(feedbackDialogOpen(useFeedbackDialog.getState())).toBe(true);
  });

  it("keeps the draft of the same sign-in", () => {
    openFeedbackDialog();
    updateFeedbackDraft({ text: "Noch offen" });
    dropStaleFeedbackDialog();
    expect(useFeedbackDialog.getState().open).toBe(true);
    expect(useFeedbackDialog.getState().draft.text).toBe("Noch offen");
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
