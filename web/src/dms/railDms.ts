// The direct messages the server rail shows as avatars above the servers
// (GEL-65): conversations with unread messages, newest first. The native
// app picks them the same way (desktop/client/src/store.rs `rail_dms`).
import { badgeOf } from "../messages/notifyLevel.ts";
import type { ReadState } from "../messages/readState.ts";
import type { DirectMessage } from "./types.ts";

/** At most this many avatars; the DM button counts the rest. */
export const RAIL_DMS_MAX = 5;

export type RailDm = { dm: DirectMessage; count: number };

/**
 * Listed DMs whose badge has a number, newest message first, at most `max`.
 * A muted one, one set to "nothing" and the one being read have none
 * (notifyLevel.ts `badgeOf`), so they stay off the rail; so does an incoming
 * message request, which is not listed until it is accepted.
 */
export function railDms(
  dms: readonly DirectMessage[] | undefined,
  rows: readonly ReadState[] | undefined,
  reading: string | null,
  now: number,
  max = RAIL_DMS_MAX,
): RailDm[] {
  if (!dms || !rows) return [];
  const byChannel = new Map(
    rows
      .filter((row) => row.server_id === null)
      .map((row) => [row.channel_id, row]),
  );
  return (
    dms
      .flatMap((dm, index) => {
        const row = byChannel.get(dm.id);
        if (!row) return [];
        const { count } = badgeOf(row, reading, now);
        return count > 0
          ? [{ dm, count, latest: row.latest_order ?? -Infinity, index }]
          : [];
      })
      // The newest message first; without one, the list's own order.
      .sort((a, b) => b.latest - a.latest || a.index - b.index)
      .slice(0, Math.max(0, max))
      .map(({ dm, count }) => ({ dm, count }))
  );
}

/**
 * Where the focus goes when the rail avatar at `index` that held it (or
 * whose menu did) leaves: the avatar now in its place, the last one when it
 * was the last, or `null` for the DM button when none is left.
 */
export function focusAfterLeave(index: number, left: number): number | null {
  return left === 0 ? null : Math.min(Math.max(0, index), left - 1);
}
