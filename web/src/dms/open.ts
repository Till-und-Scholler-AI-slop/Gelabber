import { ApiError } from "../api/client.ts";
import type { DirectMessage } from "./types.ts";

/** 404/403: this conversation is gone for the caller. */
export function isGoneError(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.code === "not_found" || error.code === "forbidden")
  );
}

/** Last-DM is only a target when the list we have actually contains it. */
export function lastDmStillListed(
  lastId: string | null,
  dms: DirectMessage[] | undefined,
): lastId is string {
  return Boolean(lastId && dms?.some((dm) => dm.id === lastId));
}

/**
 * `/d` may jump into last-DM only when the row is still in the list and
 * a detail fetch has not already failed (that failure is the other half
 * of the Redirect loop with `/d/$id`).
 */
export function shouldOpenLastDm(
  lastId: string | null,
  dms: DirectMessage[] | undefined,
  detailFailed: boolean,
): boolean {
  return lastDmStillListed(lastId, dms) && !detailFailed;
}
