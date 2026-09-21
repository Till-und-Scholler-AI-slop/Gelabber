// Fixed-height strip. Appearing/disappearing names stay inside this box
// so the message list above does not relayout.

import { useMemo } from "react";

import { useSession } from "../auth/session.ts";
import type { Member } from "../servers/types.ts";
import {
  TYPING_BAR_PX,
  formatTyping,
  typingUserIds,
  useTypingStore,
} from "../ws/live.ts";

export function TypingBar({
  channelId,
  members,
}: {
  channelId: string;
  members: Member[];
}) {
  const me = useSession((s) => s.user?.id);
  const byChannel = useTypingStore((s) => s.byChannel);
  const now = useTypingStore((s) => s.now);
  const names = useMemo(() => {
    const ids = typingUserIds(byChannel, channelId, now, me);
    const lookup = new Map(members.map((m) => [m.user_id, m.name]));
    return ids.map((id) => lookup.get(id) ?? "Jemand");
  }, [byChannel, channelId, me, members, now]);

  return (
    <div
      data-testid="typing-bar"
      aria-live="polite"
      className="flex shrink-0 items-center px-4 text-xs text-neutral-500 dark:text-neutral-400"
      style={{ height: TYPING_BAR_PX }}
    >
      <span className="truncate">{formatTyping(names)}</span>
    </div>
  );
}
