// Files sent in a server's text channels, newest first (GEL-69): the
// overview's "Geteilt". `GET /api/servers/{id}/attachments`.
import { api } from "../api/client.ts";
import type { Attachment, MessageAuthor } from "../messages/types.ts";

export type SharedFile = Attachment & {
  message_id: string;
  channel_id: string;
  author: MessageAuthor;
  /** When its message was sent. */
  created_at: string;
};

export const listSharedFiles = (
  serverId: string,
  limit: number,
  signal?: AbortSignal,
) =>
  api<SharedFile[]>(`/servers/${serverId}/attachments?limit=${limit}`, {
    signal,
  });
