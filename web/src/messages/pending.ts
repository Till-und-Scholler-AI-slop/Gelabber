// Optimistic sends and failed attempts, keyed by channel. Live outside the query
// cache so an in-flight GET cannot wipe a row the user already sees.

import { create } from "zustand";

import type { ScopeStamp } from "../auth/scope.ts";

import { asAttachmentList, type Message } from "./types.ts";

/** Stable empty list so a Zustand selector does not return a new `[]` every
 *  time a channel has no in-flight send (that looped MessagePane on mount). */
export const nonePending: Message[] = [];

export type SendStage = "presign" | "upload" | "bind";
export type SendAttempt = {
  id: string;
  channelId: string;
  stamp: ScopeStamp;
  content: string;
  file?: File;
  status: "sending" | "failed" | "uncertain";
  stage: SendStage;
  error?: string;
  uploadedId?: string;
  controller: AbortController;
};

export const noneAttempts: SendAttempt[] = [];

function revoke(message: Message): void {
  for (const attachment of asAttachmentList(message.attachments)) {
    if (attachment.preview_url?.startsWith("blob:"))
      URL.revokeObjectURL(attachment.preview_url);
  }
}

/** Mounted message panes per channel. Only a pane drops a confirmed row once
 *  its page has it, so off screen nothing would ever release that row. */
const displayed = new Map<string, number>();

type PendingState = {
  attempts: Record<string, SendAttempt>;
  byChannel: Record<string, Message[]>;
  add: (channelId: string, message: Message) => void;
  confirm: (channelId: string, tmpId: string, message: Message) => void;
  remove: (channelId: string, id: string) => void;
  clear: (channelId: string) => void;
};

export const usePendingMessages = create<PendingState>((set) => ({
  byChannel: {},
  attempts: {},
  add: (channelId, message) =>
    set((state) => ({
      byChannel: {
        ...state.byChannel,
        [channelId]: [...(state.byChannel[channelId] ?? []), message],
      },
    })),
  confirm: (channelId, tmpId, message) =>
    set((state) => ({
      byChannel: {
        ...state.byChannel,
        [channelId]: (state.byChannel[channelId] ?? []).flatMap((row) => {
          if (row.id !== tmpId) return [row];
          // Nobody sees it; the page or the next read shows the server row.
          if (!displayed.has(channelId)) {
            revoke(row);
            return [];
          }
          return [
            {
              ...message,
              attachments: asAttachmentList(message.attachments).map(
                (attachment, index) => ({
                  ...attachment,
                  preview_url: row.attachments[index]?.preview_url,
                }),
              ),
            },
          ];
        }),
      },
    })),
  remove: (channelId, id) => {
    const row = usePendingMessages
      .getState()
      .byChannel[channelId]?.find((m) => m.id === id);
    if (row) revoke(row);
    set((state) => ({
      byChannel: {
        ...state.byChannel,
        [channelId]: (state.byChannel[channelId] ?? []).filter(
          (m) => m.id !== id,
        ),
      },
    }));
  },
  clear: (channelId) =>
    set((state) => {
      if (!(channelId in state.byChannel)) return state;
      for (const row of state.byChannel[channelId] ?? []) revoke(row);
      const next = { ...state.byChannel };
      delete next[channelId];
      return { byChannel: next };
    }),
}));

/** A pane shows the overlay; leaving drops its confirmed rows and previews.
 *  Unsent and failed `tmp:` rows stay with their recoverable attempts. */
export function displayPending(channelId: string): () => void {
  displayed.set(channelId, (displayed.get(channelId) ?? 0) + 1);
  return () => {
    const count = (displayed.get(channelId) ?? 1) - 1;
    if (count > 0) {
      displayed.set(channelId, count);
      return;
    }
    displayed.delete(channelId);
    for (const row of usePendingMessages.getState().byChannel[channelId] ?? [])
      if (!row.id.startsWith("tmp:")) removePending(channelId, row.id);
  };
}

export function addPending(channelId: string, message: Message): void {
  usePendingMessages.getState().add(channelId, message);
}

/** Swap the `tmp:` row for the server message — same overlay, new id. */
export function confirmPending(
  channelId: string,
  tmpId: string,
  message: Message,
): void {
  usePendingMessages.getState().confirm(channelId, tmpId, message);
}

export function removePending(channelId: string, id: string): void {
  usePendingMessages.getState().remove(channelId, id);
}

/** Drop every in-flight optimistic row. Account switches must not keep them. */
export function resetPendingMessages(): void {
  const state = usePendingMessages.getState();
  for (const attempt of Object.values(state.attempts))
    attempt.controller.abort();
  for (const rows of Object.values(state.byChannel))
    for (const row of rows) revoke(row);
  usePendingMessages.setState({ byChannel: {}, attempts: {} });
}

export function saveAttempt(attempt: SendAttempt): void {
  usePendingMessages.setState((state) => ({
    attempts: { ...state.attempts, [attempt.id]: attempt },
  }));
}

export function discardAttempt(id: string): void {
  const attempt = usePendingMessages.getState().attempts[id];
  if (!attempt || attempt.status === "sending") return;
  removePending(attempt.channelId, id);
  forgetAttempt(id);
}

export function forgetAttempt(id: string): void {
  usePendingMessages.setState((state) => {
    const attempts = { ...state.attempts };
    delete attempts[id];
    return { attempts };
  });
}
