import { useEffect, useRef, useState, type ComponentType } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import {
  displayedReactions,
  reactionMutationOptions,
} from "../messages/reactions.ts";
import { asReactionList, type Message } from "../messages/types.ts";
import { Modal } from "./Modal.tsx";

function PickerLoader({ onSelect }: { onSelect: (emoji: string) => void }) {
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
}: {
  message: Message;
  canSend: boolean;
}) {
  const client = useQueryClient();
  const userId = useSession((state) => state.user?.id ?? "");
  const [open, setOpen] = useState(false);
  const addButton = useRef<HTMLButtonElement>(null);
  const mutation = useMutation(
    reactionMutationOptions(client, message.channel_id, message.id),
  );
  const intent =
    mutation.isPending &&
    mutation.variables &&
    stampHolds(mutation.variables.scope)
      ? mutation.variables
      : undefined;
  const reactions = displayedReactions(message.reactions, userId, intent);
  const busy = Boolean(intent);
  if (
    !userId ||
    message.id.startsWith("tmp:") ||
    (!canSend && reactions.length === 0)
  )
    return null;
  const react = (emoji: string, add: boolean) => {
    const scope = takeStamp();
    if (!scope || scope.userId !== userId || busy || (add && !canSend)) return;
    mutation.mutate({ emoji, add, scope });
    return scope;
  };
  return (
    <div
      className="mt-1 flex flex-wrap items-center gap-1.5"
      aria-label="Reaktionen"
    >
      {reactions.map(({ emoji, user_ids }) => {
        const mine = user_ids.includes(userId);
        return (
          <button
            key={emoji}
            type="button"
            aria-pressed={mine}
            aria-label={`${emoji}: ${user_ids.length} Reaktionen${mine ? ", du hast reagiert" : ""}`}
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
            className={`min-h-9 rounded-full border px-2.5 text-sm disabled:opacity-50 aria-disabled:opacity-50 ${mine ? "border-blue-500 bg-blue-50 text-blue-800 dark:bg-blue-950 dark:text-blue-200" : "border-neutral-300 dark:border-neutral-600"}`}
          >
            <span aria-hidden="true">
              {emoji} {user_ids.length}
            </span>
          </button>
        );
      })}
      {canSend && (
        <button
          ref={addButton}
          type="button"
          aria-label="Reaktion hinzufügen"
          // Transient API work must not make the modal's opener unfocusable.
          aria-disabled={busy || undefined}
          onClick={() => {
            if (!busy) setOpen(true);
          }}
          className="min-h-9 rounded-full border border-neutral-300 px-2.5 text-sm aria-disabled:opacity-50 dark:border-neutral-600"
        >
          ＋ <span aria-hidden="true">☺</span>
        </button>
      )}
      <Modal
        open={open}
        onClose={() => setOpen(false)}
        title="Reaktion auswählen"
        wide
      >
        {open && (
          <PickerLoader
            onSelect={(emoji) => {
              const mine =
                asReactionList(message.reactions)
                  .find((reaction) => reaction.emoji === emoji)
                  ?.user_ids.includes(userId) ?? false;
              react(emoji, !mine);
              setOpen(false);
            }}
          />
        )}
      </Modal>
    </div>
  );
}
