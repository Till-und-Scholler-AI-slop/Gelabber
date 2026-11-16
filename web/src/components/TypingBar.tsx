// Fixed-height strip. Appearing/disappearing names stay inside this box
// so the message list above does not relayout.

import { useMemo } from "react";

import { useSession } from "../auth/session.ts";
import { useBlockedIds } from "../blocks/queries.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
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
  const t = useT(chat);
  const me = useSession((s) => s.user?.id);
  const byChannel = useTypingStore((s) => s.byChannel);
  const now = useTypingStore((s) => s.now);
  const blocked = useBlockedIds();
  const names = useMemo(() => {
    // Someone the user blocks types unseen (#565).
    const ids = typingUserIds(byChannel, channelId, now, me).filter(
      (id) => !blocked.has(id),
    );
    const lookup = new Map(members.map((m) => [m.user_id, m.name]));
    return ids.map((id) => lookup.get(id) ?? t("typing.someone"));
  }, [blocked, byChannel, channelId, me, members, now, t]);

  return (
    <div
      data-testid="typing-bar"
      aria-live="polite"
      className="lr-typing-bar"
      style={{ height: TYPING_BAR_PX }}
    >
      {names.length > 0 ? (
        <span className="lr-typing-dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      ) : null}
      <span className="truncate">{formatTyping(names)}</span>
    </div>
  );
}
