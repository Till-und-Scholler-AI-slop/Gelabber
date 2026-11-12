// The dialog's limits, and the API's refusals in its words. The API checks
// the same; these only say it earlier.

import { ApiError } from "../api/client.ts";
import { fieldMessage } from "../auth/rules.ts";
import { translate } from "../i18n/catalog.ts";
import { feedback } from "../i18n/catalogs/feedback.ts";

export const TEXT_MAX = 4000;
/** From this many characters, the dialog counts down. */
export const COUNTER_FROM = 3600;
/** Fewer left than this, the count is announced. */
export const COUNTER_LIVE_BELOW = 100;
/** `IMAGE_MAX_BYTES` of the API: 5 MiB. */
export const IMAGE_MAX_BYTES = 5 * 1024 * 1024;
export const IMAGE_TYPES = ["image/png", "image/jpeg", "image/webp"] as const;

/** Characters left once the counter shows, else null. */
export function charactersLeft(text: string): number | null {
  return text.length >= COUNTER_FROM ? TEXT_MAX - text.length : null;
}

/** Why an image cannot go along, or null when it can. */
export function imageProblem(image: {
  type: string;
  size: number;
}): "wrongType" | "tooLarge" | null {
  if (!(IMAGE_TYPES as readonly string[]).includes(image.type))
    return "wrongType";
  if (image.size > IMAGE_MAX_BYTES) return "tooLarge";
  return null;
}

type ClipboardItem = {
  kind: string;
  type: string;
  getAsFile: () => File | null;
};

/** The clipboard's first image, if it holds one. Its text is left to the
 * browser, so pasting text still works. */
export function clipboardImage(
  items: ArrayLike<ClipboardItem> | null | undefined,
): File | null {
  const item = Array.from(items ?? []).find(
    (entry) => entry.kind === "file" && entry.type.startsWith("image/"),
  );
  return item?.getAsFile() ?? null;
}

/**
 * A failed send in the dialog's words, under its fields: the text, the
 * image (a 413 is the size). Null when it is no field's: the form says it.
 */
export function sendFieldErrors(error: unknown): Record<string, string> | null {
  if (!(error instanceof ApiError)) return null;
  if (error.status === 413)
    return { image: translate(feedback, "image.tooLarge") };
  if (error.code !== "validation_failed") return null;
  const fields: Record<string, string> = {};
  const text = error.fields.text;
  if (text === "required") fields.text = translate(feedback, "text.required");
  else if (text === "too_long")
    fields.text = translate(feedback, "text.tooLong");
  else if (text === "too_many")
    fields.text = translate(feedback, "thread.full");
  else if (text) fields.text = fieldMessage("text", text);
  if (error.fields.image) fields.image = translate(feedback, "image.invalid");
  return Object.keys(fields).length > 0 ? fields : null;
}
