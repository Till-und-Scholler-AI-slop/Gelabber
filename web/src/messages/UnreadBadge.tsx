import { useState } from "react";

import { Badge as Pill } from "../components/ui/Badge.tsx";
import { badgeSeen } from "./badgeEntrance.ts";
import { useReadState } from "./readState.ts";
import { useBadge, type BadgeScope } from "./useBadge.ts";

/** A red number for mentions and direct messages, a dot for other unread
 * messages (notifyLevel.ts). Lists that mark unread rows themselves pass
 * `showDot={false}`: the dot is then only read out. */
export function UnreadBadge({
  id,
  rail = false,
  showDot = true,
  className,
  ...scope
}: BadgeScope & {
  id?: string;
  rail?: boolean;
  showDot?: boolean;
  className?: string;
}) {
  const { count, dot } = useBadge(scope);
  // A badge that appears while the list is on screen scales in once.
  const loaded = useReadState().data !== undefined;
  const shown = count > 0 || dot;
  const [seen, setSeen] = useState(() => badgeSeen(null, loaded, shown));
  if (seen.loaded !== loaded || seen.shown !== shown)
    setSeen(badgeSeen(seen, loaded, shown));
  const fresh = seen.fresh ? "is-fresh" : undefined;
  if (count > 0)
    return (
      <Pill
        tone="count"
        className={[
          rail ? "unread-count unread-count--rail" : "unread-count",
          fresh,
          className,
        ]
          .filter(Boolean)
          .join(" ")}
        id={id}
        // A named, non-live element: rail links reference it as their
        // description, while a live region would announce every count change.
        role="img"
        aria-label={`${count > 99 ? "Mehr als 99" : count} Benachrichtigungen`}
      >
        {count > 99 ? "99+" : count}
      </Pill>
    );
  if (!dot) return null;
  if (!showDot)
    return (
      <span id={id} className="sr-only">
        Ungelesene Nachrichten
      </span>
    );
  return (
    <span
      className={["unread-dot", fresh, className].filter(Boolean).join(" ")}
      id={id}
      role="img"
      aria-label="Ungelesene Nachrichten"
    />
  );
}
