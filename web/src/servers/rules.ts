// Client-side mirror of `api/src/servers/validate.rs`.

import type { FieldCode } from "../api/client.ts";
import type { ChannelKind } from "./types.ts";

export const SERVER_NAME_MAX = 100;
export const INVITE_MAX_USES_MAX = 10_000;
export const INVITE_EXPIRES_HOURS_MAX = 24 * 365;

const chars = (value: string): number => Array.from(value).length;
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f]/;

/** Server and category names. */
export function validateName(raw: string): FieldCode | null {
  const value = raw.trim();
  if (value.length === 0) return "required";
  if (chars(value) > SERVER_NAME_MAX) return "too_long";
  if (CONTROL.test(value)) return "invalid";
  return null;
}

/**
 * What the server will store for a text channel name: lower-case, spaces
 * and underscores become `-`, runs collapse, no `#`, no edge dashes. Shown
 * live under the input so nobody is surprised by the result.
 */
export function slugifyChannelName(raw: string): string {
  let out = "";
  let dash = true;
  for (const c of raw.trim()) {
    if (/\s/.test(c) || c === "_" || c === "-") {
      if (!dash) {
        out += "-";
        dash = true;
      }
    } else if (c === "#") {
      continue;
    } else {
      out += c.toLowerCase();
      dash = false;
    }
  }
  return out.replace(/-+$/, "");
}

export function validateChannelName(
  raw: string,
  kind: ChannelKind,
): FieldCode | null {
  const base = validateName(raw);
  if (base) return base;
  if (kind === "text" && slugifyChannelName(raw).length === 0) return "invalid";
  return null;
}

/** Empty = unlimited; otherwise 1…10 000. */
export function validateMaxUses(raw: string): FieldCode | null {
  if (raw.trim() === "") return null;
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1 || n > INVITE_MAX_USES_MAX)
    return "invalid";
  return null;
}

/** Empty = never; otherwise whole hours, 1…8760. */
export function validateExpiresInHours(raw: string): FieldCode | null {
  if (raw.trim() === "") return null;
  const n = Number(raw);
  if (!Number.isInteger(n) || n < 1 || n > INVITE_EXPIRES_HOURS_MAX)
    return "invalid";
  return null;
}
