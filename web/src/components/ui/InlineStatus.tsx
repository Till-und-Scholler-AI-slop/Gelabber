import { CheckIcon, WarningIcon } from "../Icons.tsx";
import type { InlineStatusValue } from "./useInlineStatus.ts";
import "./ui.css";

/**
 * A status line that is always mounted, so screen readers announce what is
 * written into it ("Gespeichert.", "Link kopiert"). Empty, it takes no room.
 */
export function InlineStatus({
  status,
  className,
}: {
  status: InlineStatusValue;
  className?: string;
}) {
  return (
    <span
      role="status"
      className={[
        "ui-inline-status",
        status ? `ui-inline-status--${status.tone}` : undefined,
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {status?.tone === "success" ? <CheckIcon size={14} /> : null}
      {status?.tone === "error" ? <WarningIcon size={14} /> : null}
      {status?.message}
    </span>
  );
}
