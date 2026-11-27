import { api } from "../api/client.ts";
import { isMessage, malformed, wholePage, wholeRows } from "../api/wire.ts";
import type { Message } from "./types.ts";

export type MessageContext = {
  target_id: string;
  messages: Message[];
  before: string;
  after: string;
};

export function messageContext(
  channelId: string,
  messageId: string,
  signal?: AbortSignal,
) {
  return api<unknown>(`/channels/${channelId}/messages/${messageId}/context`, {
    signal,
  }).then((value): MessageContext => {
    const context = value as Partial<MessageContext> | null;
    if (
      !context ||
      typeof context.target_id !== "string" ||
      typeof context.before !== "string" ||
      typeof context.after !== "string"
    )
      throw malformed("message context", value);
    return {
      ...context,
      messages: wholeRows(context.messages, isMessage, "message"),
    } as MessageContext;
  });
}

export function searchMessages(
  channelId: string,
  q: string,
  before?: string,
  signal?: AbortSignal,
) {
  const params = new URLSearchParams({ q, limit: "25" });
  if (before) params.set("before", before);
  return api<unknown>(`/channels/${channelId}/messages/search?${params}`, {
    signal,
  }).then(wholePage);
}
