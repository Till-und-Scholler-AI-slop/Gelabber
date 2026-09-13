// Client-side mirror of `api/src/messages/validate.rs`.

import type { FieldCode } from "../api/client.ts";

export const CONTENT_MAX = 2000;
export const PAGE_SIZE = 50;

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
