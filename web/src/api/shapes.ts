// The building blocks of the checks in `wire.ts`, and the check of the
// signed-in account, which the API client itself needs: kept apart from
// `wire.ts` so that the client imports no module that imports it.

import type { User } from "../auth/types.ts";
import { isLocale } from "../i18n/locale.ts";

export type Check<T> = (value: unknown) => value is T;
export type Fields = Record<string, unknown>;

export const record = (value: unknown): value is Fields =>
  value !== null && typeof value === "object" && !Array.isArray(value);
export const text = (value: unknown): value is string =>
  typeof value === "string";
export const textOrNull = (value: unknown) => value === null || text(value);
export const count = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);
export const countOrNull = (value: unknown) => value === null || count(value);
export const flag = (value: unknown): value is boolean =>
  typeof value === "boolean";
export const texts = (value: unknown): value is string[] =>
  Array.isArray(value) && value.every(text);
export const listOf = (value: unknown, check: (item: unknown) => boolean) =>
  Array.isArray(value) && value.every(check);
/** An optional field: absent, or what `check` accepts. */
export const absentOr = (value: unknown, check: (value: unknown) => boolean) =>
  value === undefined || check(value);

/** The signed-in account, as the session and `/me` answer it. */
export const isUser: Check<User> = (value): value is User =>
  record(value) &&
  text(value.id) &&
  value.id.length > 0 &&
  text(value.email) &&
  text(value.name) &&
  textOrNull(value.avatar_url) &&
  text(value.created_at) &&
  isLocale(value.locale);

const reported = new Set<string>();

/** Says once per object what was dropped, with the object for the reader of
 *  the console. */
export function reportMalformed(what: string, value: unknown): void {
  const id =
    record(value) && (text(value.id) || text(value.channel_id))
      ? String(value.id ?? value.channel_id)
      : JSON.stringify(value)?.slice(0, 200);
  const key = `${what}:${id}`;
  if (reported.has(key)) return;
  reported.add(key);
  console.warn(`[gelabber] dropped a malformed ${what} from the server`, value);
}

/** Forgets what was reported, for tests. */
export function resetMalformedReports(): void {
  reported.clear();
}
