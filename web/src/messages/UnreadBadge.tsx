import { badgeOf, sumBadges } from "./notifyLevel.ts";
import { useMuteClock } from "./notificationSettings.ts";
import { useReadState, useReadingChannel } from "./readState.ts";

/** A red number for mentions and direct messages, a dot for other unread
 * messages (notifyLevel.ts). */
export function UnreadBadge({
  id,
  channelId,
  serverId,
  dms = false,
  rail = false,
}: {
  id?: string;
  channelId?: string;
  serverId?: string;
  dms?: boolean;
  rail?: boolean;
}) {
  const reading = useReadingChannel();
  const now = useMuteClock();
  const { data } = useReadState();
  const { count, dot } = sumBadges(
    (data ?? [])
      .filter((row) =>
        channelId
          ? row.channel_id === channelId
          : dms
            ? row.server_id === null
            : row.server_id === serverId,
      )
      .map((row) => badgeOf(row, reading, now)),
  );
  if (count > 0)
    return (
      <span
        className={`${rail ? "absolute -right-1 -bottom-1" : "ml-auto"} shrink-0 rounded-full bg-[var(--lr-danger)] px-1.5 py-0.5 text-xs font-semibold text-white`}
        id={id}
        // A named, non-live element: rail links reference it as their
        // description, while a live region would announce every count change.
        role="img"
        aria-label={`${count > 99 ? "Mehr als 99" : count} Benachrichtigungen`}
      >
        {count > 99 ? "99+" : count}
      </span>
    );
  if (!dot) return null;
  return (
    <span
      className={`${rail ? "absolute -right-0.5 -bottom-0.5 h-3 w-3 ring-2 ring-[var(--lr-rail)]" : "ml-auto h-2 w-2"} shrink-0 rounded-full bg-[var(--lr-text)]`}
      id={id}
      role="img"
      aria-label="Ungelesene Nachrichten"
    />
  );
}
