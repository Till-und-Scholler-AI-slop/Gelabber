// The edits open in a conversation, by message id. They live outside the
// rows: a row scrolled out of the virtual list's overscan unmounts, and would
// take the editor and what was typed in it along.

import { create } from "zustand";

import { scopeGeneration } from "../auth/scope.ts";

type EditDraftState = {
  byMessage: Record<string, { channelId: string; text: string }>;
};

export const useEditDrafts = create<EditDraftState>(() => ({ byMessage: {} }));

/** The text of the message's open edit; undefined while it is not edited. */
export function useEditDraft(messageId: string): string | undefined {
  return useEditDrafts((state) => state.byMessage[messageId]?.text);
}

/** Opens or updates the message's edit; null closes it. */
export function setEditDraft(
  channelId: string,
  messageId: string,
  text: string | null,
): void {
  useEditDrafts.setState((state) => {
    const byMessage = { ...state.byMessage };
    if (text === null) delete byMessage[messageId];
    else byMessage[messageId] = { channelId, text };
    return { byMessage };
  });
}

/** The conversation was left: its open edits close. */
export function clearEditDrafts(channelId: string): void {
  useEditDrafts.setState((state) => ({
    byMessage: Object.fromEntries(
      Object.entries(state.byMessage).filter(
        ([, draft]) => draft.channelId !== channelId,
      ),
    ),
  }));
}

/** Account switch: an open edit must not follow into another account. */
export function resetEditDrafts(): void {
  useEditDrafts.setState({ byMessage: {} });
}

type EditMutation = {
  mutateAsync: (input: { id: string; content: string }) => Promise<unknown>;
};

/**
 * A row's save, with `failed` run if the server turns it down. Not a per-call
 * onError: one mutation serves every row, and TanStack hands that only to
 * its latest call, so a second save would drop the first one's recovery.
 */
export function saveEdit(edit: EditMutation, id: string) {
  return (content: string, failed: () => void): void => {
    // A save that fails after an account switch stays closed: reopening it
    // would show the old account's text to the next one.
    const generation = scopeGeneration();
    void edit.mutateAsync({ id, content }).catch(() => {
      if (scopeGeneration() === generation) failed();
    });
  };
}
