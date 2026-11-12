import { useId, type ReactNode } from "react";

import { InlineStatus } from "./ui/InlineStatus.tsx";
import type { InlineStatusValue } from "./ui/useInlineStatus.ts";

/** Title and hint on the left, the row's button on the right. */
export function ActionRow({
  title,
  badge,
  hint,
  status,
  action,
  children,
}: {
  title: string;
  /** A tag after the title, such as "Dieses Gerät". */
  badge?: ReactNode;
  hint: string;
  status: InlineStatusValue;
  action: (describedBy: string) => ReactNode;
  children?: ReactNode;
}) {
  const id = useId();
  return (
    <li className="settings-action-row">
      <div className="settings-action-row__text">
        <span id={`${id}-title`} className="settings-action-row__title">
          {title}
          {badge}
        </span>
        <span id={`${id}-hint`} className="settings-hint">
          {hint}
        </span>
        <InlineStatus status={status} />
      </div>
      {action(`${id}-title ${id}-hint`)}
      {children}
    </li>
  );
}
