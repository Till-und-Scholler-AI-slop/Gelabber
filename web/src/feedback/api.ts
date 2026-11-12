// One function per feedback route. No caching here; `queries.ts` owns that.

import { api } from "../api/client.ts";
import { uploadTimeoutMs } from "../messages/api.ts";
import type {
  FeedbackEntry,
  FeedbackMessage,
  FeedbackPage,
  FeedbackStatus,
  NewFeedback,
} from "./types.ts";

export const feedbackStatus = (signal?: AbortSignal) =>
  api<FeedbackStatus>("/feedback/status", { signal });

/** Stages one image for the entry about to be created: only on "Senden",
 * since an unclaimed upload expires after ten minutes. The bytes go as
 * they are, with the time a large file needs on a slow line. */
export const uploadFeedbackImage = (image: Blob) =>
  api<{ image: string }>("/feedback/images", {
    method: "POST",
    body: image,
    timeoutMs: uploadTimeoutMs(image.size),
  });

export const createFeedback = (input: NewFeedback) =>
  api<FeedbackEntry>("/feedback", { method: "POST", body: input });

export const listFeedback = (offset: number, signal?: AbortSignal) =>
  api<FeedbackPage>(offset > 0 ? `/feedback?offset=${offset}` : "/feedback", {
    signal,
  });

export const replyFeedback = (id: string, text: string) =>
  api<FeedbackMessage>(`/feedback/${id}/messages`, {
    method: "POST",
    body: { text },
  });

/** The entry's operator replies are seen: its `unread` clears. */
export const readFeedback = (id: string) =>
  api<null>(`/feedback/${id}/read`, { method: "POST" });

/** Uploads the image when there is one, then creates the entry. */
export async function sendFeedback(
  input: Omit<NewFeedback, "image">,
  image: Blob | null,
): Promise<FeedbackEntry> {
  const staged = image ? (await uploadFeedbackImage(image)).image : null;
  return createFeedback(staged ? { ...input, image: staged } : input);
}
