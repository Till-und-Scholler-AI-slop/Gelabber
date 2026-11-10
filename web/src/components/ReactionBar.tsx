import { useRef } from "react";
import { createPortal } from "react-dom";
import { displayedReactions, reactorNames } from "../messages/reactions.ts";
import { useMentions } from "./mentionContext.ts";
import type { Reacting } from "./useReaction.ts";
import type { Message } from "../messages/types.ts";
import { SmileIcon } from "./Icons.tsx";
import { Modal } from "./Modal.tsx";
import { EmojiPickerPart } from "./emojiPickerPart.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { emoji as emojiCatalog } from "../i18n/catalogs/emoji.ts";

export function EmojiPickerLoader({
  onSelect,
}: {
  onSelect: (emoji: string) => void;
}) {
  const t = useT(emojiCatalog);
  return (
    <EmojiPickerPart.Component
      onSelect={onSelect}
      fallback={<p role="status">{t("picker.loading")}</p>}
    />
  );
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
  const t = useT(chat);
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
      aria-label={t("reactions.label")}
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
            aria-label={t(
              mine ? "reactions.chipMineLabel" : "reactions.chipLabel",
              { emoji, count: user_ids.length },
            )}
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
                aria-label={t("reactions.add")}
                title={t("reactions.add")}
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
        title={t("reactions.pick")}
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
