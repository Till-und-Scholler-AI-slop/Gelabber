// What a conversation's unread state and notification settings (#241, #242)
// mean for its badge, the totals outside the page and its notifications.
// The server sends the effective level and mute with every unread row
// (api/src/messages/workflows.rs); nothing here asks for more.
import type { ReadState } from "./readState.ts";

export type NotifyLevel = "all" | "mentions" | "none";
type Row = Pick<
  ReadState,
  | "channel_id"
  | "server_id"
  | "unread_count"
  | "mention_count"
  | "level"
  | "muted"
  | "muted_until"
>;

export function levelOf(row: Pick<Row, "level"> | undefined): NotifyLevel {
  return row?.level === "mentions" || row?.level === "none" ? row.level : "all";
}

/** A mute that holds at `now` (server clock): one with an end that has
 * passed is over without waiting for the next snapshot. */
export function isMuted(
  row: Pick<Row, "muted" | "muted_until"> | undefined,
  now: number,
): boolean {
  if (!row?.muted) return false;
  if (!row.muted_until) return true;
  const until = Date.parse(row.muted_until);
  return Number.isNaN(until) || until > now;
}

export type Badge = {
  /** A red number: mentions, or unread direct messages. */
  count: number;
  /** Unread without a number: a plain dot. */
  dot: boolean;
};

/**
 * Discord's rules. In a server channel ordinary unread messages are a dot
 * and only mentions get a number; a direct message always counts. A mute
 * takes the dot (and a DM's number) away; a mention still counts unless the
 * level is "nothing". The conversation being read shows nothing.
 */
export function badgeOf(row: Row, reading: string | null, now: number): Badge {
  if (row.channel_id === reading) return { count: 0, dot: false };
  const muted = isMuted(row, now);
  const none = levelOf(row) === "none";
  const unread = Math.max(0, row.unread_count);
  const dot = !muted && !none && unread > 0;
  if (row.server_id === null) return { count: muted || none ? 0 : unread, dot };
  return {
    count: none ? 0 : Math.max(0, row.mention_count ?? 0),
    dot,
  };
}

export function sumBadges(badges: Iterable<Badge>): Badge {
  let count = 0;
  let dot = false;
  for (const badge of badges) {
    count += badge.count;
    dot ||= badge.dot;
  }
  return { count, dot };
}

/** Whether a new message of the conversation is announced (toast, system
 * notification, sound, push). A conversation without a row yet notifies. */
export function wantsNotification(
  row: Pick<Row, "level" | "muted" | "muted_until"> | undefined,
  message: { dm: boolean; mentioned: boolean },
  now: number,
): boolean {
  if (isMuted(row, now)) return false;
  const level = levelOf(row);
  if (level === "none") return false;
  if (level === "mentions") return message.dm || message.mentioned;
  return true;
}
