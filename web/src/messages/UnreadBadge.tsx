import { useReadState, useReadingChannel } from "./readState.ts";

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
  const { data } = useReadState();
  const count = (data ?? []).reduce(
    (total, row) =>
      total +
      ((
        channelId
          ? row.channel_id === channelId
          : dms
            ? row.server_id === null
            : row.server_id === serverId
      )
        ? row.channel_id === reading
          ? 0
          : row.unread_count
        : 0),
    0,
  );
  if (!count) return null;
  return (
    <span
      className={`${rail ? "absolute -right-1 -bottom-1" : "ml-auto"} shrink-0 rounded-full bg-[var(--lr-accent)] px-1.5 py-0.5 text-xs font-semibold text-[var(--lr-accent-ink)]`}
      id={id}
      role="status"
      aria-label={`${count > 99 ? "Mehr als 99" : count} ungelesene Nachrichten`}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
