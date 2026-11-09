// The edits open in a conversation, by message id. They live outside the
// rows: a row scrolled out of the virtual list's overscan unmounts, and would
// take the editor and what was typed in it along.

import { create } from "zustand";

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
