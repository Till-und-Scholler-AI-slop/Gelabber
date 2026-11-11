// Client-side mirror of `backend/api/src/directory/validate.rs`.

import type { FieldCode } from "../api/client.ts";
import type { DirectoryLanguage, JoinMode, Listing } from "./types.ts";

export const DESCRIPTION_MAX = 300;
export const MESSAGE_MAX = 300;
export const QUERY_MAX = 100;
/** The directory answers this many listings at a time. */
export const PAGE_SIZE = 30;
/** Typing in the search waits this long before it asks. */
export const SEARCH_DEBOUNCE_MS = 250;

const chars = (value: string): number => Array.from(value).length;
// Rust's `char::is_control` (C0, DEL and C1), which also keeps it one line.
// eslint-disable-next-line no-control-regex
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/** One line of at most `max` characters, trimmed; empty is allowed. */
function line(raw: string, max: number): FieldCode | null {
  const value = raw.trim();
  if (chars(value) > max) return "too_long";
  if (CONTROL.test(value)) return "invalid";
  return null;
}

/** A listing's description: required once the server is listed. */
export function validateDescription(
  raw: string,
  listed: boolean,
): FieldCode | null {
  if (listed && raw.trim().length === 0) return "required";
  return line(raw, DESCRIPTION_MAX);
}

/** The optional message of a request to join. */
export function validateMessage(raw: string): FieldCode | null {
  return line(raw, MESSAGE_MAX);
}

/** A search: empty means none; otherwise 1–100 characters on one line. */
export function validateQuery(raw: string): FieldCode | null {
  return line(raw, QUERY_MAX);
}

/** One of the languages `GET /api/directory` names. */
export function validateLanguage(
  code: string | null | undefined,
  languages: readonly Pick<DirectoryLanguage, "code">[],
): FieldCode | null {
  if (!code) return "required";
  return languages.some((language) => language.code === code)
    ? null
    : "invalid";
}

export function isJoinMode(value: unknown): value is JoinMode {
  return value === "open" || value === "request";
}

/** What a row's button offers the caller. */
export type ListingAction =
  | "open" // a member: "Öffnen"
  | "join" // an open server: "Beitreten"
  | "request" // "Beitritt anfragen"
  | "requested" // pending: "Angefragt", disabled
  | "declined"; // the cooldown: "Nicht angenommen", disabled

export function listingAction(
  listing: Pick<Listing, "member" | "join_mode" | "request">,
): ListingAction {
  if (listing.member) return "open";
  // An open server lets anyone in, also someone who asked while it was not.
  if (listing.join_mode === "open") return "join";
  if (listing.request === "pending") return "requested";
  if (listing.request === "declined") return "declined";
  return "request";
}
