import type { KeyboardEvent, ReactNode, RefObject } from "react";

import {
  CopyIcon,
  PencilIcon,
  PlusIcon,
  ReplyIcon,
  TrashIcon,
} from "./Icons.tsx";
import { Popover } from "./ui/Popover.tsx";

/** The quick row of the sheet, in this order. */
const QUICK_REACTIONS = ["👍", "❤️", "😂", "😮", "😢", "🎉"] as const;

/** What can be done with one message; a missing entry is not offered. */
export type MessageActions = {
  react?: (emoji: string) => void;
  /** The full emoji picker. */
  moreReactions?: () => void;
  reply?: () => void;
  edit?: () => void;
  copy?: () => void;
  /** `skipConfirm` with Shift held, as on desktop. */
  remove?: (skipConfirm: boolean) => void;
};

/** Moves between the entries with the arrow keys, Pos1 and Ende. */
function moveFocus(event: KeyboardEvent<HTMLElement>) {
  const entries = [
    ...event.currentTarget.querySelectorAll<HTMLElement>(
      '[role="menuitem"]:not(:disabled)',
    ),
  ];
  const at = entries.indexOf(document.activeElement as HTMLElement);
  const next =
    event.key === "ArrowDown" || event.key === "ArrowRight"
      ? (at + 1) % entries.length
      : event.key === "ArrowUp" || event.key === "ArrowLeft"
        ? (at - 1 + entries.length) % entries.length
        : event.key === "Home"
          ? 0
          : event.key === "End"
            ? entries.length - 1
            : null;
  if (next === null || entries.length === 0) return;
  event.preventDefault();
  entries[next]?.focus();
}

/**
 * The entries: a row of quick reactions, then reply, edit, copy, and
 * delete last. `mine` marks the reactions the viewer already gave; picking
 * one of those takes it back. Every entry closes the menu through `onDone`.
 */
export function MessageActionItems({
  actions,
  mine = [],
  onDone,
}: {
  actions: MessageActions;
  mine?: readonly string[];
  onDone: () => void;
}) {
  const run = (action: (() => void) | undefined) => () => {
    onDone();
    action?.();
  };
  const row = (label: string, icon: ReactNode, action?: () => void) =>
    action ? (
      <button
        type="button"
        role="menuitem"
        className="lr-message-menu-item"
        onClick={run(action)}
      >
        {icon}
        {label}
      </button>
    ) : null;
  return (
    <div className="lr-message-menu" onKeyDown={moveFocus}>
      {actions.react ? (
        <div
          className="lr-message-menu-reactions"
          role="group"
          aria-label="Reagieren"
        >
          {QUICK_REACTIONS.map((emoji) => {
            const given = mine.includes(emoji);
            return (
              <button
                key={emoji}
                type="button"
                role="menuitem"
                aria-label={
                  given
                    ? `Reaktion ${emoji} entfernen`
                    : `Mit ${emoji} reagieren`
                }
                className={`lr-message-menu-emoji${given ? " is-mine" : ""}`}
                onClick={run(() => actions.react?.(emoji))}
              >
                <span aria-hidden="true">{emoji}</span>
              </button>
            );
          })}
          {actions.moreReactions ? (
            <button
              type="button"
              role="menuitem"
              aria-label="Andere Reaktion auswählen"
              title="Andere Reaktion auswählen"
              className="lr-message-menu-emoji"
              onClick={run(actions.moreReactions)}
            >
              <PlusIcon size={18} />
            </button>
          ) : null}
        </div>
      ) : null}
      {row("Antworten", <ReplyIcon size={18} />, actions.reply)}
      {row("Bearbeiten", <PencilIcon size={18} />, actions.edit)}
      {row("Text kopieren", <CopyIcon size={18} />, actions.copy)}
      {actions.remove ? (
        <>
          <hr className="lr-message-menu-divider" />
          <button
            type="button"
            role="menuitem"
            className="lr-message-menu-item is-danger"
            title="Löschen (Umschalt: ohne Nachfrage)"
            onClick={(event) => run(() => actions.remove?.(event.shiftKey))()}
          >
            <TrashIcon size={18} />
            Löschen
          </button>
        </>
      ) : null}
    </div>
  );
}

/**
 * A message's actions: a menu at the "Mehr" button or the right click on
 * desktop, a bottom sheet after a long press on a phone.
 */
export function MessageActionSheet({
  open,
  onClose,
  anchor,
  author,
  actions,
  mine,
}: {
  open: boolean;
  onClose: () => void;
  anchor: RefObject<HTMLElement | null>;
  author: string;
  actions: MessageActions;
  mine?: readonly string[];
}) {
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchor={anchor}
      label={`Nachricht von ${author}`}
      placement="bottom-end"
      role="menu"
      className="lr-message-menu-host"
    >
      <MessageActionItems actions={actions} mine={mine} onDone={onClose} />
    </Popover>
  );
}
