import { ApiError, api, timeoutSignal } from "../api/client.ts";
import { isMessage, whole, wholePage } from "../api/wire.ts";
import type { GifPick } from "../gifs/api.ts";
import { translate } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import type {
  ListMessagesParams,
  Message,
  PresignRequest,
  PresignResponse,
} from "./types.ts";

/** A message the server answered with, whole, or the request fails. */
export const wholeMessage = (value: unknown): Message =>
  whole(value, isMessage, "message");

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
  api<unknown>(`/channels/${channelId}/messages${query(params)}`, {
    signal,
  }).then(wholePage);

export const createMessage = (
  channelId: string,
  content: string,
  attachmentIds: string[] = [],
  gif?: GifPick,
  replyTo?: string,
  nonce?: string,
  /** Alt texts by attachment id (#575). */
  descriptions: Record<string, string> = {},
) =>
  api<unknown>(`/channels/${channelId}/messages`, {
    method: "POST",
    body: {
      ...(gif
        ? { content: "", gif: { slug: gif.slug, query: gif.query } }
        : {
            content,
            attachment_ids: attachmentIds,
            ...(Object.keys(descriptions).length > 0
              ? { attachment_descriptions: descriptions }
              : {}),
          }),
      ...(replyTo ? { reply_to: replyTo } : {}),
      ...(nonce ? { nonce } : {}),
    },
  }).then(wholeMessage);

export const updateMessage = (id: string, content: string) =>
  api<unknown>(`/messages/${id}`, { method: "PATCH", body: { content } }).then(
    wholeMessage,
  );

/** Sets (or, empty, clears) a picture's alt text; the text stays (#575). */
export const updateAltText = (
  messageId: string,
  attachmentId: string,
  text: string,
) =>
  api<unknown>(`/messages/${messageId}`, {
    method: "PATCH",
    body: { attachment_descriptions: { [attachmentId]: text.trim() || null } },
  }).then(wholeMessage);

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

/** Slowest uplink an upload must still finish on: 256 kbit/s, in bytes/ms. */
const UPLOAD_MIN_BYTES_PER_MS = 32;

/**
 * Two minutes plus the time `size` bytes take on a slow uplink: fetch
 * reports no upload progress, so a stall cannot be told from a slow line.
 */
export function uploadTimeoutMs(size: number): number {
  return 120_000 + Math.ceil(size / UPLOAD_MIN_BYTES_PER_MS);
}

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
  onProgress?: (share: number) => void,
): Promise<void> {
  // Only XMLHttpRequest reports upload progress (#574).
  if (onProgress && typeof XMLHttpRequest !== "undefined")
    return putWithProgress(url, file, headers, onProgress, signal);
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
      signal: timeoutSignal(uploadTimeoutMs(file.size), signal),
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "TimeoutError") {
      throw new Error(translate(chat, "upload.timedOut"), { cause: error });
    }
    if (error instanceof DOMException && error.name === "AbortError") {
      throw new Error(translate(chat, "upload.aborted"), { cause: error });
    }
    throw new ApiError("network", 0, "Network error.");
  }
  if (!response.ok) {
    throw new Error(translate(chat, "upload.failed"));
  }
}

/** `putPresigned` through XMLHttpRequest, with the same errors. */
function putWithProgress(
  url: string,
  file: File,
  headers: Record<string, string>,
  onProgress: (share: number) => void,
  signal?: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const aborted = () =>
      reject(
        new Error(translate(chat, "upload.aborted"), {
          cause: new DOMException("Aborted", "AbortError"),
        }),
      );
    if (signal?.aborted) return aborted();
    const xhr = new XMLHttpRequest();
    xhr.open("PUT", url);
    for (const [name, value] of Object.entries(headers)) {
      if (name.toLowerCase() === "content-length") continue;
      xhr.setRequestHeader(name, value);
    }
    xhr.timeout = uploadTimeoutMs(file.size);
    xhr.upload.onprogress = (event) => {
      if (event.lengthComputable && event.total > 0)
        onProgress(Math.min(1, event.loaded / event.total));
    };
    const stop = () => xhr.abort();
    signal?.addEventListener("abort", stop, { once: true });
    const done = () => signal?.removeEventListener("abort", stop);
    xhr.onload = () => {
      done();
      if (xhr.status >= 200 && xhr.status < 300) {
        onProgress(1);
        resolve();
      } else reject(new Error(translate(chat, "upload.failed")));
    };
    xhr.ontimeout = () => {
      done();
      reject(new Error(translate(chat, "upload.timedOut")));
    };
    xhr.onabort = () => {
      done();
      aborted();
    };
    xhr.onerror = () => {
      done();
      reject(new ApiError("network", 0, "Network error."));
    };
    xhr.send(file);
  });
}

export function attachmentUrl(id: string): string {
  return `/api/attachments/${id}`;
}

/** A picture's thumbnail with that long edge (#574). Until the server has
 *  made it, the same link answers with the original, uncached. */
export function thumbnailUrl(id: string, edge: 400 | 800): string {
  return `/api/attachments/${id}?size=${edge}`;
}

/** The same file, answered with `Content-Disposition: attachment`: a
 * browser ignores `<a download>` after the redirect to MinIO and would
 * otherwise navigate away from the app (and end a call). */
export function attachmentDownloadUrl(id: string): string {
  return `/api/attachments/${id}?download=1`;
}
