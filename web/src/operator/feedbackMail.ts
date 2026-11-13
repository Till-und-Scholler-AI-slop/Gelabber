// Betrieb → Feedback: the mail state of an operator's reply to a visitor is
// set once the send is done, so the page asks again until it is.

import type { OperatorFeedback } from "./types.ts";

/** How long a reply's mail may read "pending" before the page stops asking. */
export const MAIL_PENDING_WATCH_MS = 120_000;
/** How often the page asks again meanwhile. */
export const MAIL_PENDING_REFETCH_MS = 3_000;

/**
 * Every few seconds while a recent reply's mail is still being sent, so
 * "Mail verschickt" or "Mail nicht zugestellt" follows by itself; false
 * otherwise. A reply left pending by a restart is not asked about forever.
 */
export function mailPendingRefetch(
  entries: readonly Pick<OperatorFeedback, "messages">[],
  now: number = Date.now(),
): number | false {
  const waiting = entries.some((entry) =>
    entry.messages.some(
      (message) =>
        message.mail === "pending" &&
        now - Date.parse(message.created_at) < MAIL_PENDING_WATCH_MS,
    ),
  );
  return waiting ? MAIL_PENDING_REFETCH_MS : false;
}
