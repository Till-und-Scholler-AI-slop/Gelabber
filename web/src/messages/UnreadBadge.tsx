import { useReadState } from "./readState.ts";

export function UnreadBadge({
  channelId,
  serverId,
  dms = false,
  rail = false,
}: {
  channelId?: string;
  serverId?: string;
  dms?: boolean;
  rail?: boolean;
}) {
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
        ? row.unread_count
        : 0),
    0,
  );
  if (!count) return null;
  return (
    <span
      className={`${rail ? "absolute -right-1 -bottom-1" : "ml-auto"} shrink-0 rounded-full bg-[var(--lr-accent)] px-1.5 py-0.5 text-xs font-semibold text-[var(--lr-accent-ink)]`}
      aria-label={`${count} ungelesene Nachrichten`}
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
