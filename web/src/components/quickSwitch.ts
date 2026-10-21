// What the quick switcher (Strg+K) offers and in which order, apart from
// the DOM so it can be tested in node.

import type { DirectMessage } from "../dms/types.ts";
import type { Badge } from "../messages/notifyLevel.ts";
import type { Server, ServerDetail } from "../servers/types.ts";
import { channelOrder } from "../servers/rows.ts";

export type SwitchTarget =
  | { kind: "server"; serverId: string }
  | { kind: "channel"; serverId: string; channelId: string }
  | { kind: "dm"; channelId: string };

export type SwitchEntry = {
  id: string;
  kind: "server" | "text" | "voice" | "dm";
  label: string;
  /** Where it is: the server of a channel. */
  detail: string | null;
  target: SwitchTarget;
  badge: Badge;
  /** A direct message's picture. */
  avatar?: string | null;
};

const NO_BADGE: Badge = { count: 0, dot: false };

/**
 * Every server, the channels of the servers whose details are loaded and
 * every direct message. The conversation open right now is left out.
 */
export function switchEntries({
  servers,
  details,
  dms,
  badges,
  current,
}: {
  servers: readonly Server[];
  details: readonly ServerDetail[];
  dms: readonly DirectMessage[];
  /** By channel id. */
  badges: ReadonlyMap<string, Badge>;
  current: string | undefined;
}): SwitchEntry[] {
  const entries: SwitchEntry[] = [];
  for (const dm of dms) {
    if (dm.id === current) continue;
    entries.push({
      id: `dm:${dm.id}`,
      kind: "dm",
      label: dm.peer.name,
      detail: "Direktnachricht",
      target: { kind: "dm", channelId: dm.id },
      badge: badges.get(dm.id) ?? NO_BADGE,
      avatar: dm.peer.avatar_url,
    });
  }
  for (const server of servers) {
    const detail = details.find((row) => row.id === server.id);
    let count = 0;
    let dot = false;
    for (const channel of detail ? channelOrder(detail) : []) {
      const badge = badges.get(channel.id) ?? NO_BADGE;
      count += badge.count;
      dot ||= badge.dot;
      if (channel.id === current || channel.id.startsWith("tmp:")) continue;
      entries.push({
        id: `channel:${channel.id}`,
        kind: channel.kind,
        label: channel.name,
        detail: server.name,
        target: { kind: "channel", serverId: server.id, channelId: channel.id },
        badge,
      });
    }
    entries.push({
      id: `server:${server.id}`,
      kind: "server",
      label: server.name,
      detail: "Server",
      target: { kind: "server", serverId: server.id },
      badge: { count, dot },
    });
  }
  return entries;
}

/** Lower case, without accents and a leading "#" or "@". */
function normalise(text: string): string {
  return text
    .normalize("NFD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/^[#@]/, "")
    .trim();
}

/**
 * How well `label` matches `query`, higher is better, or `null`: the start
 * of the name beats the start of a word, which beats anywhere inside,
 * which beats the letters in order with gaps ("agm" finds "allgemein").
 */
export function matchScore(query: string, label: string): number | null {
  const q = normalise(query);
  const text = normalise(label);
  if (!q) return 0;
  if (text.startsWith(q)) return 4;
  if (new RegExp(`(?:^|[\\s_\\-.])${escape(q)}`).test(text)) return 3;
  if (text.includes(q)) return 2;
  let at = 0;
  for (const char of q) {
    at = text.indexOf(char, at);
    if (at < 0) return null;
    at += 1;
  }
  return 1;
}

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

const unreadRank = (badge: Badge) => (badge.count > 0 ? 2 : badge.dot ? 1 : 0);

/**
 * The entries that match, best first; unread ones lead among equals, so
 * the empty query lists what is new. At most `limit`.
 */
export function rankEntries(
  entries: readonly SwitchEntry[],
  query: string,
  limit = 50,
): SwitchEntry[] {
  return entries
    .flatMap((entry, index) => {
      const score = matchScore(query, entry.label);
      return score === null ? [] : [{ entry, score, index }];
    })
    .sort(
      (a, b) =>
        b.score - a.score ||
        unreadRank(b.entry.badge) - unreadRank(a.entry.badge) ||
        b.entry.badge.count - a.entry.badge.count ||
        a.index - b.index,
    )
    .slice(0, limit)
    .map(({ entry }) => entry);
}
