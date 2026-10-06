import { api } from "../api/client.ts";
import type { MessagePage } from "./types.ts";
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
  return api<MessageContext>(
    `/channels/${channelId}/messages/${messageId}/context`,
    { signal },
  );
}

export function searchMessages(
  channelId: string,
  q: string,
  before?: string,
  signal?: AbortSignal,
) {
  const params = new URLSearchParams({ q, limit: "25" });
  if (before) params.set("before", before);
  return api<MessagePage>(`/channels/${channelId}/messages/search?${params}`, {
    signal,
  });
}
