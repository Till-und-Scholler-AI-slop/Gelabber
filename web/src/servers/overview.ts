// The server overview's choices (GEL-69): it shows only what is shown
// nowhere else. Who is in a call is the sidebar's, who is online the member
// list's; the overview has the Lives, what was missed and what was shared.
import type { ReadState } from "../messages/readState.ts";
import type { Badge } from "../messages/notifyLevel.ts";
import type { Message } from "../messages/types.ts";
import type { Channel } from "./types.ts";

/** The greeting's key by the local hour: morning from 5, day from 11,
 * evening from 18 and through the night. */
export function greetingKey(
  hour: number,
):
  | "overview.greetingMorning"
  | "overview.greetingDay"
  | "overview.greetingEvening" {
  if (hour >= 5 && hour < 11) return "overview.greetingMorning";
  if (hour >= 11 && hour < 18) return "overview.greetingDay";
  return "overview.greetingEvening";
}

/** At most this many channel cards under "Verpasst". */
export const CARDS_MAX = 4;
/** A channel this busy without a mention is shown dimmed, with one message. */
export const BUSY_UNREAD = 10;
/** Messages a card shows: one channel alone gets more room. */
export const SHOWN_ALONE = 5;
export const SHOWN_CARD = 2;
export const SHOWN_BUSY = 1;
/** Files and pictures under "Geteilt". */
export const SHARED_MAX = 4;
/** Asked for beyond those, so what "Verpasst" shows can be left out. */
export const SHARED_ASKED = 12;

export type MissedCard = {
  channel: Channel;
  unread: number;
  mentioned: boolean;
  /** Newest message of the channel when the counts were taken. */
  latest: number | null;
  /** The read position: messages after it are unread. */
  readOrder: number | null;
  busy: boolean;
};

/** Text channels with something unread, as the sidebar marks them: the ones
 * that mention me first, then the newest message, then the sidebar's order.
 * The first `CARDS_MAX` become cards; `hidden` counts the rest. */
export function missedCards(
  channels: readonly Channel[],
  rows: ReadonlyMap<string, ReadState>,
  badges: ReadonlyMap<string, Badge>,
): { cards: MissedCard[]; hidden: number } {
  const all = channels
    .map((channel, index) => ({ channel, index }))
    .filter(({ channel }) => {
      if (channel.kind !== "text" || channel.id.startsWith("tmp:"))
        return false;
      const badge = badges.get(channel.id);
      return Boolean(badge && (badge.count > 0 || badge.dot));
    })
    .map(({ channel, index }) => {
      const row = rows.get(channel.id);
      const unread = Math.max(0, row?.unread_count ?? 0);
      const mentioned = (badges.get(channel.id)?.count ?? 0) > 0;
      return {
        index,
        card: {
          channel,
          unread,
          mentioned,
          latest: row?.latest_order ?? null,
          readOrder: row?.read_order ?? null,
          busy: !mentioned && unread >= BUSY_UNREAD,
        },
      };
    })
    .sort(
      (a, b) =>
        Number(b.card.mentioned) - Number(a.card.mentioned) ||
        (b.card.latest ?? -Infinity) - (a.card.latest ?? -Infinity) ||
        a.index - b.index,
    )
    .map(({ card }) => card);
  return {
    cards: all.slice(0, CARDS_MAX),
    hidden: Math.max(0, all.length - CARDS_MAX),
  };
}

/** How many of a card's newest unread messages it shows. */
export function shownCount(card: MissedCard, alone: boolean): number {
  if (alone) return SHOWN_ALONE;
  return card.busy ? SHOWN_BUSY : SHOWN_CARD;
}

/** The newest `count` unread ones of a page (oldest first, as the list
 * endpoint answers): those after the read position, where it is known. */
export function unreadTail(
  page: readonly Message[],
  readOrder: number | null,
  count: number,
): Message[] {
  const unread =
    readOrder === null
      ? page
      : page.filter(
          (message) =>
            message.created_order === undefined ||
            message.created_order > readOrder,
        );
  return unread.slice(Math.max(0, unread.length - count));
}

/** Whether a message reaches me: named, `@everyone`, or `@here` as
 * stored for me. */
export function mentionsMe(
  message: Pick<Message, "mentions" | "mention_everyone">,
  me: string,
): boolean {
  return message.mention_everyone === true || !!message.mentions?.includes(me);
}

const PICTURES = new Set([
  "image/jpeg",
  "image/png",
  "image/gif",
  "image/webp",
]);
export const isPicture = (contentType: string) => PICTURES.has(contentType);

/** "Geteilt": the newest files, without those of messages "Verpasst"
 * already shows. */
export function sharedShown<T extends { message_id: string }>(
  files: readonly T[],
  shownMessages: ReadonlySet<string>,
  max = SHARED_MAX,
): T[] {
  return files
    .filter((file) => !shownMessages.has(file.message_id))
    .slice(0, max);
}
