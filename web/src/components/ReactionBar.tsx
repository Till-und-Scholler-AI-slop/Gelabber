import { useEffect, useRef, useState, type ComponentType } from "react";
import { createPortal } from "react-dom";
import { displayedReactions, reactorNames } from "../messages/reactions.ts";
import { useMentions } from "./mentionContext.ts";
import type { Reacting } from "./useReaction.ts";
import type { Message } from "../messages/types.ts";
import { SmileIcon } from "./Icons.tsx";
import { Modal } from "./Modal.tsx";

export function EmojiPickerLoader({
  onSelect,
}: {
  onSelect: (emoji: string) => void;
}) {
  const [load, setLoad] = useState<{
    Picker?: ComponentType<{ onSelect: (emoji: string) => void }>;
    failed?: boolean;
  }>({});
  useEffect(() => {
    let alive = true;
    void import("./EmojiPicker.tsx").then(
      (module) => {
        if (alive) setLoad({ Picker: module.default });
      },
      () => {
        if (alive) setLoad({ failed: true });
      },
    );
    return () => {
      alive = false;
    };
  }, []);
  if (load.Picker) return <load.Picker onSelect={onSelect} />;
  if (load.failed)
    return (
      <div role="alert" className="flex flex-col gap-3">
        <p>Die Emoji-Auswahl konnte nicht geladen werden.</p>
        <button
          type="button"
          className="rounded border px-3 py-2"
          onClick={() => window.location.reload()}
        >
          Seite neu laden
        </button>
      </div>
    );
  return <p role="status">Emoji-Auswahl wird geladen…</p>;
}

export function ReactionBar({
  message,
  canSend,
  reaction,
  addSlot,
}: {
  message: Message;
  canSend: boolean;
  reaction: Reacting;
  /** The row's hover toolbar; the add button renders there when given. */
  addSlot?: HTMLElement | null;
}) {
  const {
    userId,
    intent,
    busy,
    react,
    toggle,
    picking,
    setPicking,
    popped,
    popDone,
  } = reaction;
  const { members } = useMentions();
  const addButton = useRef<HTMLButtonElement>(null);
  const reactions = displayedReactions(message.reactions, userId, intent);
  if (
    !userId ||
    message.id.startsWith("tmp:") ||
    (!canSend && reactions.length === 0)
  )
    return null;
  const nameOf = (id: string) =>
    members.find((member) => member.id === id)?.name;
  return (
    <div
      className={`lr-reactions${reactions.length === 0 ? " is-empty" : ""}`}
      aria-label="Reaktionen"
    >
      {reactions.map(({ emoji, user_ids }) => {
        const mine = user_ids.includes(userId);
        // As the tooltip, also the chip's description for screen readers.
        const who = reactorNames(user_ids, nameOf, userId);
        return (
          <button
            key={emoji}
            type="button"
            aria-pressed={mine}
            aria-label={`${emoji}: ${user_ids.length} ${user_ids.length === 1 ? "Reaktion" : "Reaktionen"}${mine ? ", du hast reagiert" : ""}`}
            title={who ? who[0]!.toUpperCase() + who.slice(1) : undefined}
            disabled={!mine && !canSend}
            aria-disabled={busy || undefined}
            onClick={(event) => {
              const focused = document.activeElement === event.currentTarget;
              const accepted = react(emoji, !mine);
              if (accepted && mine && user_ids.length === 1 && focused) {
                // Move focus while this last chip still exists, before the
                // optimistic removal. No late response can overwrite focus.
                const fallback = canSend
                  ? addButton.current
                  : event.currentTarget.closest<HTMLElement>(".lr-message-row");
                fallback?.focus({ preventScroll: true });
              }
            }}
            className={`lr-reaction${mine ? " is-mine" : ""}${popped === emoji ? " is-popping" : ""}`}
            onAnimationEnd={popped === emoji ? popDone : undefined}
          >
            <span aria-hidden="true">
              {emoji} {user_ids.length}
            </span>
          </button>
        );
      })}
      {canSend
        ? (() => {
            const button = (
              <button
                ref={addButton}
                type="button"
                aria-label="Reaktion hinzufügen"
                title="Reaktion hinzufügen"
                // Transient API work must not make the modal's opener unfocusable.
                aria-disabled={busy || undefined}
                onClick={() => {
                  if (!busy) setPicking(true);
                }}
                className={addSlot ? "lr-message-action" : "lr-reaction"}
              >
                <SmileIcon size={addSlot ? 16 : 15} />
              </button>
            );
            return addSlot ? createPortal(button, addSlot) : button;
          })()
        : null}
      <Modal
        open={picking}
        onClose={() => setPicking(false)}
        title="Reaktion auswählen"
        wide
      >
        {picking && (
          <EmojiPickerLoader
            onSelect={(emoji) => {
              toggle(emoji);
              setPicking(false);
            }}
          />
        )}
      </Modal>
    </div>
  );
}
