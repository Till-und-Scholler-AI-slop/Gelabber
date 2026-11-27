// The server overview's choices (GEL-69, GEL-75): who is online is the
// member list's; the overview has who is in which call with the way in, the
// Lives, what was missed and what was shared.
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
  return message.mention_everyone === true || message.mentions.includes(me);
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

/** "Im Call": at most this many people per voice channel, then "+n". */
export const CALL_PEOPLE_SHOWN = 6;

export type CallRoom = {
  channel: Channel;
  /** Who sits there, in the roster's order. */
  people: string[];
  /** Someone there shares a screen. */
  live: boolean;
  /** The call the user is in. */
  mine: boolean;
};

/** The voice channels someone sits in, for "Im Call" (GEL-75): the user's
 * own call first, then the fullest, then the sidebar's order. `order` is the
 * sidebar's channels, `seats` user → their seat, `live` channel → user →
 * what they share, `mine` the channel of the user's call here or null. */
export function callRooms(
  order: readonly Channel[],
  seats: Readonly<Record<string, { channelId: string }>>,
  live: Readonly<Record<string, Readonly<Record<string, { s?: true }>>>>,
  mine: string | null,
): CallRoom[] {
  const people = new Map<string, string[]>();
  for (const [userId, seat] of Object.entries(seats)) {
    const list = people.get(seat.channelId) ?? [];
    list.push(userId);
    people.set(seat.channelId, list);
  }
  return order
    .flatMap((channel, index) => {
      const inside = channel.kind === "voice" ? people.get(channel.id) : null;
      if (!inside?.length) return [];
      const sharing = Object.values(live[channel.id] ?? {});
      return [
        {
          index,
          room: {
            channel,
            people: inside,
            live: sharing.some((kinds) => kinds.s === true),
            mine: channel.id === mine,
          },
        },
      ];
    })
    .sort(
      (a, b) =>
        Number(b.room.mine) - Number(a.room.mine) ||
        b.room.people.length - a.room.people.length ||
        a.index - b.index,
    )
    .map(({ room }) => room);
}
