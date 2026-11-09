// The small icon buttons at the end of a navigation row (a channel, a
// category, a conversation): edit, add, and "…" for the row's menu.
import type { MouseEvent, ReactNode } from "react";

import { ROW_ITEM } from "./rowKeys.ts";

// On a touch screen a row's actions open with a long press, as the hint
// under the list says; the "…" button stays for screen readers and shows
// once a keyboard reaches it.
export const TOUCH_ACTIONS = "sr-only focus-within:not-sr-only";

// With a mouse a row's actions show on hover, while focus is in the row and
// while its menu is open; index.css takes them out of the accessibility
// tree while hidden.
export const HOVER_ACTIONS =
  "sidebar-row-actions flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100 group-[.is-menu-open]:opacity-100";

export function RowActionButton({
  label,
  onClick,
  children,
  small,
  tabIndex,
  describedBy,
  expanded,
}: {
  label: string;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  children: ReactNode;
  small?: boolean;
  /** -1 for a row item that is not its row's Tab stop right now. */
  tabIndex?: number;
  describedBy?: string;
  /** Set on a button that opens a menu: whether it is open. */
  expanded?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-describedby={describedBy}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      tabIndex={tabIndex}
      onClick={onClick}
      {...ROW_ITEM}
      className={[
        "shell-icon-button",
        small ? "sidebar-action-small" : "",
      ].join(" ")}
    >
      {children}
    </button>
  );
}
