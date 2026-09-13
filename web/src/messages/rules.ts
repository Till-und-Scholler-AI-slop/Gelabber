// Client-side mirror of `api/src/messages/validate.rs` and
// `api/src/attachments/validate.rs`.

import type { FieldCode } from "../api/client.ts";

export const CONTENT_MAX = 2000;
export const PAGE_SIZE = 50;
export const SIZE_MAX = 25 * 1024 * 1024;
export const FILENAME_MAX = 255;

export const ALLOWED_TYPES = [
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
  "application/pdf",
  "text/plain",
  "application/zip",
  "audio/mpeg",
  "audio/wav",
  "video/mp4",
] as const;

const IMAGE_TYPES = new Set([
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
]);

const EXT_TYPE: Record<string, string> = {
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  png: "image/png",
  gif: "image/gif",
  webp: "image/webp",
  pdf: "application/pdf",
  txt: "text/plain",
  zip: "application/zip",
  mp3: "audio/mpeg",
  wav: "audio/wav",
  mp4: "video/mp4",
};

const chars = (value: string): number => Array.from(value).length;

/** Same normalisation as the API: CRLF → LF, trim, then the length check. */
export function normaliseContent(raw: string): string {
  return raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim();
}

export function validateContent(raw: string): FieldCode | null {
  const value = normaliseContent(raw);
  if (value.length === 0) return "required";
  if (chars(value) > CONTENT_MAX) return "too_long";
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(value))
    return "invalid";
  return null;
}

export function inferContentType(file: { name: string; type: string }): string {
  const typed = file.type.trim().toLowerCase().split(";")[0]?.trim() ?? "";
  if (typed) return typed;
  const ext = file.name.split(".").pop()?.toLowerCase() ?? "";
  return EXT_TYPE[ext] ?? "";
}

export function isImageType(contentType: string): boolean {
  return IMAGE_TYPES.has(contentType);
}

export function validateAttachment(file: {
  name: string;
  type: string;
  size: number;
}): { field: "filename" | "content_type" | "size"; code: FieldCode } | null {
  const name = file.name.replace(/^.*[/\\]/, "").trim();
  if (!name) return { field: "filename", code: "required" };
  if (chars(name) > FILENAME_MAX)
    return { field: "filename", code: "too_long" };
  const type = inferContentType(file);
  if (!type) return { field: "content_type", code: "required" };
  if (!(ALLOWED_TYPES as readonly string[]).includes(type)) {
    return { field: "content_type", code: "invalid" };
  }
  if (file.size <= 0) return { field: "size", code: "required" };
  if (file.size > SIZE_MAX) return { field: "size", code: "too_long" };
  return null;
}
