import { api } from "../api/client.ts";
import type { ListMessagesParams, Message, MessagePage } from "./types.ts";

function query(params: ListMessagesParams): string {
  const search = new URLSearchParams();
  if (params.before) search.set("before", params.before);
  if (params.after) search.set("after", params.after);
  if (params.limit !== undefined) search.set("limit", String(params.limit));
  const encoded = search.toString();
  return encoded.length === 0 ? "" : `?${encoded}`;
}

export const listMessages = (
  channelId: string,
  params: ListMessagesParams = {},
  signal?: AbortSignal,
) =>
  api<MessagePage>(`/channels/${channelId}/messages${query(params)}`, {
    signal,
  });

export const createMessage = (channelId: string, content: string) =>
  api<Message>(`/channels/${channelId}/messages`, {
    method: "POST",
    body: { content },
  });

export const updateMessage = (id: string, content: string) =>
  api<Message>(`/messages/${id}`, { method: "PATCH", body: { content } });

export const deleteMessage = (id: string) =>
  api<null>(`/messages/${id}`, { method: "DELETE" });
