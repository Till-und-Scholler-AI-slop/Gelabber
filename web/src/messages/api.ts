import { ApiError, api } from "../api/client.ts";
import type {
  ListMessagesParams,
  Message,
  MessagePage,
  PresignRequest,
  PresignResponse,
} from "./types.ts";

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

export const createMessage = (
  channelId: string,
  content: string,
  attachmentIds: string[] = [],
) =>
  api<Message>(`/channels/${channelId}/messages`, {
    method: "POST",
    body: { content, attachment_ids: attachmentIds },
  });

export const updateMessage = (id: string, content: string) =>
  api<Message>(`/messages/${id}`, { method: "PATCH", body: { content } });

export const deleteMessage = (id: string) =>
  api<null>(`/messages/${id}`, { method: "DELETE" });

export const presignAttachment = (channelId: string, body: PresignRequest) =>
  api<PresignResponse>(`/channels/${channelId}/attachments`, {
    method: "POST",
    body,
  });

const UPLOAD_TIMEOUT_MS = 120_000;

/** PUT the file at the presigned MinIO URL. Not same-origin — no cookies. */
export async function putPresigned(
  url: string,
  file: File,
  headers: Record<string, string>,
): Promise<void> {
  let response: Response;
  try {
    const putHeaders = new Headers();
    for (const [name, value] of Object.entries(headers)) {
      // Forbidden request header; the browser (and curl) set it from the body.
      // The URL is signed for this length — a bigger body will not match.
      if (name.toLowerCase() === "content-length") continue;
      putHeaders.set(name, value);
    }
    response = await fetch(url, {
      method: "PUT",
      headers: putHeaders,
      body: file,
      signal: AbortSignal.timeout(UPLOAD_TIMEOUT_MS),
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "TimeoutError") {
      throw new Error(
        "Der Upload hat zu lange gedauert und wurde abgebrochen.",
        { cause: error },
      );
    }
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error("Der Upload wurde abgebrochen.", { cause: error });
    }
    throw new ApiError("network", 0, "Network error.");
  }
  if (!response.ok) {
    throw new Error("Der Upload ist fehlgeschlagen.");
  }
}

export function attachmentUrl(id: string): string {
  return `/api/attachments/${id}`;
}
