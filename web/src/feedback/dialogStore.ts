// The one "Feedback geben" dialog and its draft. The user menu, the settings
// area and the dialog's own link share them, and closing keeps the draft
// (kind, text and image) until it is sent or the page reloads.

import { create } from "zustand";

import { scopeGeneration } from "../auth/scope.ts";
import type { FeedbackKind } from "./types.ts";

export type FeedbackDraft = {
  kind: FeedbackKind;
  text: string;
  image: File | null;
  /** An object URL of `image` for the thumbnail, owned by this store. */
  preview: string | null;
};

type FeedbackDialogState = {
  open: boolean;
  draft: FeedbackDraft;
  /** The signed-in identity the draft belongs to (`scopeGeneration`). */
  generation: number;
};

export const EMPTY_DRAFT: FeedbackDraft = {
  kind: "idea",
  text: "",
  image: null,
  preview: null,
};

function release(draft: FeedbackDraft): void {
  if (draft.preview) URL.revokeObjectURL(draft.preview);
}

export const useFeedbackDialog = create<FeedbackDialogState>(() => ({
  open: false,
  draft: EMPTY_DRAFT,
  generation: scopeGeneration(),
}));

/** Opens the dialog; a draft from an earlier sign-in is dropped. */
export function openFeedbackDialog(): void {
  const generation = scopeGeneration();
  const state = useFeedbackDialog.getState();
  if (state.generation === generation) {
    useFeedbackDialog.setState({ open: true });
    return;
  }
  release(state.draft);
  useFeedbackDialog.setState({ open: true, draft: EMPTY_DRAFT, generation });
}

/** The dialog is open, for the identity signed in now. */
export function feedbackDialogOpen(state: FeedbackDialogState): boolean {
  return state.open && state.generation === scopeGeneration();
}

/**
 * Drops what an earlier sign-in left: a dialog open when its session ended,
 * and its draft. The next account's dialog then starts closed and empty.
 */
export function dropStaleFeedbackDialog(): void {
  const generation = scopeGeneration();
  const state = useFeedbackDialog.getState();
  if (state.generation === generation) return;
  release(state.draft);
  useFeedbackDialog.setState({ open: false, draft: EMPTY_DRAFT, generation });
}

/** Closes the dialog; the draft stays. */
export function closeFeedbackDialog(): void {
  useFeedbackDialog.setState({ open: false });
}

export function updateFeedbackDraft(
  patch: Partial<Pick<FeedbackDraft, "kind" | "text">>,
): void {
  useFeedbackDialog.setState((state) => ({
    draft: { ...state.draft, ...patch },
  }));
}

/** Attaches `image` in place of any earlier one; null removes it. */
export function setFeedbackImage(image: File | null): void {
  const { draft } = useFeedbackDialog.getState();
  release(draft);
  useFeedbackDialog.setState({
    draft: {
      ...draft,
      image,
      preview: image ? URL.createObjectURL(image) : null,
    },
  });
}

/** Sent: the next dialog starts empty. */
export function clearFeedbackDraft(): void {
  release(useFeedbackDialog.getState().draft);
  useFeedbackDialog.setState({ draft: EMPTY_DRAFT });
}
