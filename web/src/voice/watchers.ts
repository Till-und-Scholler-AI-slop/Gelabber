// Who watches a Live (#521): the gateway's `sig t:"w"` sets from the roster,
// and the own watch from local state so it does not wait for the echo.

import type { VoiceFlags } from "./roster.ts";

export type Watchers = { watching: string[]; notWatching: string[] };

/**
 * The occupants of `channelId` other than `owner`, split by whether they
 * watch `owner`'s Live. In occupancy order; the caller sorts by name.
 */
export function watchersOf(args: {
  owner: string;
  channelId: string;
  /** `byServer[serverId]` of the voice roster. */
  occupancy: Record<string, VoiceFlags>;
  /** `watches[serverId]` of the voice roster. */
  watches: Record<string, readonly string[]>;
  me?: string;
  /** The own `sourceSubscriptions`. */
  myWatches?: Record<string, Partial<Record<"s", boolean>>>;
}): Watchers {
  const { owner, channelId, occupancy, watches, me, myWatches } = args;
  const watching: string[] = [];
  const notWatching: string[] = [];
  for (const [userId, flags] of Object.entries(occupancy)) {
    if (userId === owner || flags.channelId !== channelId) continue;
    const isWatching =
      userId === me
        ? myWatches?.[owner]?.s === true
        : (watches[userId]?.includes(owner) ?? false);
    (isWatching ? watching : notWatching).push(userId);
  }
  return { watching, notWatching };
}
