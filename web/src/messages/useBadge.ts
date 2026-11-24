// Unread badges as the lists show them (notifyLevel.ts), read from the
// read state.
import { badgeOf, sumBadges, type Badge } from "./notifyLevel.ts";
import { useMuteClock } from "./notificationSettings.ts";
import { useReadState, useReadingChannel } from "./readState.ts";

export type BadgeScope = {
  channelId?: string;
  serverId?: string;
  dms?: boolean;
  /** Conversations left out of the sum: the DM button skips those the rail
   * shows as avatars (GEL-65). */
  except?: ReadonlySet<string>;
};

/** The badge of one conversation, a whole server or all direct messages
 * (notifyLevel.ts): muted ones and the one being read count for nothing. */
export function useBadge({
  channelId,
  serverId,
  dms = false,
  except,
}: BadgeScope): Badge {
  const reading = useReadingChannel();
  const now = useMuteClock();
  const { data } = useReadState();
  return sumBadges(
    (data ?? [])
      .filter((row) => !except?.has(row.channel_id))
      .filter((row) =>
        channelId
          ? row.channel_id === channelId
          : dms
            ? row.server_id === null
            : row.server_id === serverId,
      )
      .map((row) => badgeOf(row, reading, now)),
  );
}

/** Every conversation's badge, by channel id. */
export function useBadges(): ReadonlyMap<string, Badge> {
  const reading = useReadingChannel();
  const now = useMuteClock();
  const { data } = useReadState();
  return new Map(
    (data ?? []).map((row) => [row.channel_id, badgeOf(row, reading, now)]),
  );
}
