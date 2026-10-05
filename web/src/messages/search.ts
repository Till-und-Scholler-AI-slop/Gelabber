import { api } from "../api/client.ts";
import type { MessagePage } from "./types.ts";

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
