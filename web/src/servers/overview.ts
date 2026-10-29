import { readableMentions, type MentionMember } from "../messages/mentions.ts";
import type { Badge } from "../messages/notifyLevel.ts";
import { asSystem, systemText } from "../messages/system.ts";
import {
  asAttachmentList,
  replyExcerpt,
  type Message,
} from "../messages/types.ts";
import type { VoiceFlags } from "../voice/roster.ts";
import type { Channel, ServerDetail } from "./types.ts";

type Connection = {
  status: string;
  serverId: string | null;
  channelId: string | null;
};

const isVoice = (channel: Channel) =>
  channel.kind === "voice" && !channel.id.startsWith("tmp:");

/** Prefer the current call, then the last visited room, then an occupied room. */
export function overviewRoom(
  server: Pick<ServerDetail, "id" | "channels" | "members">,
  roster: Record<string, VoiceFlags>,
  connection: Connection,
  remembered?: string,
): Channel | undefined {
  const rooms = server.channels.filter(isVoice);
  const active =
    connection.status === "joined" && connection.serverId === server.id
      ? rooms.find((c) => c.id === connection.channelId)
      : undefined;
  const knownMembers = new Set(server.members.map((member) => member.user_id));
  const occupied = new Set(
    Object.entries(roster)
      .filter(([id]) => knownMembers.has(id))
      .map(([, flags]) => flags.channelId),
  );
  return (
    active ??
    rooms.find((c) => c.id === remembered) ??
    rooms.find((c) => occupied.has(c.id)) ??
    rooms[0]
  );
}

/** "4 Mitglieder · 2 im Sprachkanal": who belongs, and who of them is in
 * one of the server's voice channels right now. The own call counts as
 * soon as it is joined, before the roster echoes it. */
export function overviewLine(
  server: Pick<ServerDetail, "id" | "channels" | "members">,
  roster: Record<string, VoiceFlags>,
  connection: Connection,
  selfId: string,
): string {
  const rooms = new Set(server.channels.filter(isVoice).map((c) => c.id));
  const where = new Map<string, string>();
  for (const member of server.members) {
    const channelId =
      member.user_id === selfId
        ? connection.status === "joined" && connection.serverId === server.id
          ? connection.channelId
          : null
        : roster[member.user_id]?.channelId;
    if (channelId && rooms.has(channelId)) where.set(member.user_id, channelId);
  }
  const count = server.members.length;
  const members = `${count} ${count === 1 ? "Mitglied" : "Mitglieder"}`;
  if (where.size === 0) return members;
  const spread = new Set(where.values()).size > 1;
  return `${members} · ${where.size} ${spread ? "in Sprachkanälen" : "im Sprachkanal"}`;
}

/** Text channels with something unread, as the sidebar marks them: the
 * newest message first, then the sidebar's order. */
export function unreadChannels(
  channels: readonly Channel[],
  badges: ReadonlyMap<string, Badge>,
  latest: ReadonlyMap<string, number | null | undefined>,
): Channel[] {
  return channels
    .map((channel, index) => ({ channel, index }))
    .filter(({ channel }) => {
      if (channel.kind !== "text" || channel.id.startsWith("tmp:"))
        return false;
      const badge = badges.get(channel.id);
      return Boolean(badge && (badge.count > 0 || badge.dot));
    })
    .sort(
      (a, b) =>
        (latest.get(b.channel.id) ?? -Infinity) -
          (latest.get(a.channel.id) ?? -Infinity) || a.index - b.index,
    )
    .map(({ channel }) => channel);
}

/** One line on a message: who wrote it and what, as a reply quotes it
 * (spoilers hidden, mentions as names). A system message names its
 * person itself. */
export function messagePreview(
  message: Pick<
    Message,
    "author" | "content" | "attachments" | "gif" | "system"
  >,
  members: readonly MentionMember[],
  selfId: string,
): { author: string | null; text: string } {
  const system = asSystem(message.system);
  if (system)
    return { author: null, text: systemText(system, message.author.name) };
  const text =
    readableMentions(replyExcerpt(message.content), members) ||
    (message.gif
      ? "GIF"
      : asAttachmentList(message.attachments).length > 0
        ? "Anhang"
        : "");
  return {
    author: message.author.id === selfId ? "Du" : message.author.name,
    text,
  };
}

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() &&
  a.getMonth() === b.getMonth() &&
  a.getDate() === b.getDate();

/** "18:28" today, "Gestern", then "21.10." ("21.10.25" in another year). */
export function shortWhen(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return "";
  if (sameDay(date, now))
    return new Intl.DateTimeFormat("de-DE", {
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (sameDay(date, yesterday)) return "Gestern";
  return new Intl.DateTimeFormat("de-DE", {
    day: "2-digit",
    month: "2-digit",
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: "2-digit" }),
  }).format(date);
}
