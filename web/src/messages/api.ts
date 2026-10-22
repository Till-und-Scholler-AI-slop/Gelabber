import { ApiError, api } from "../api/client.ts";
import type { GifPick } from "../gifs/api.ts";
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
  gif?: GifPick,
  replyTo?: string,
  nonce?: string,
) =>
  api<Message>(`/channels/${channelId}/messages`, {
    method: "POST",
    body: {
      ...(gif
        ? { content: "", gif: { slug: gif.slug, query: gif.query } }
        : { content, attachment_ids: attachmentIds }),
      ...(replyTo ? { reply_to: replyTo } : {}),
      ...(nonce ? { nonce } : {}),
    },
  });

export const updateMessage = (id: string, content: string) =>
  api<Message>(`/messages/${id}`, { method: "PATCH", body: { content } });

export const deleteMessage = (id: string) =>
  api<null>(`/messages/${id}`, { method: "DELETE" });

/** Removes the message's link previews for good (author or moderator). */
export const removePreviews = (id: string) =>
  api<null>(`/messages/${id}/previews`, { method: "DELETE" });

export const presignAttachment = (channelId: string, body: PresignRequest) =>
  api<PresignResponse>(`/channels/${channelId}/attachments`, {
    method: "POST",
    body,
  });

const UPLOAD_TIMEOUT_MS = 120_000;

/** PUT the file at the upload link from the presign answer, with every
 * header it lists. Usually a signed MinIO URL on another origin, so no
 * cookies go along; with `ATTACHMENT_DELIVERY=proxy` the API's own
 * `/api/attachments/{id}/upload`, where the same-origin session cookie and
 * the listed CSRF header authorise the upload (#348). */
export async function putPresigned(
  url: string,
  file: File,
  headers: Record<string, string>,
  signal?: AbortSignal,
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
      signal: signal
        ? AbortSignal.any([signal, AbortSignal.timeout(UPLOAD_TIMEOUT_MS)])
        : AbortSignal.timeout(UPLOAD_TIMEOUT_MS),
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

/** The same file, answered with `Content-Disposition: attachment`: a
 * browser ignores `<a download>` after the redirect to MinIO and would
 * otherwise navigate away from the app (and end a call). */
export function attachmentDownloadUrl(id: string): string {
  return `/api/attachments/${id}?download=1`;
}
