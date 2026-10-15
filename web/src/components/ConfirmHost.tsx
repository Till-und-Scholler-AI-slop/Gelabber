import { useId } from "react";

import { useConfirm, type ConfirmRequest } from "./confirm.ts";
import { dialogInitialFocus } from "./dialogFocus.ts";
import { GhostButton, Modal } from "./Modal.tsx";

export function ConfirmHost() {
  const pending = useConfirm((state) => state.pending);
  const answer = useConfirm((state) => state.answer);
  return <ConfirmDialog pending={pending} answer={answer} />;
}

export function ConfirmDialog({
  pending,
  answer,
}: {
  pending: (ConfirmRequest & { id: number }) | null;
  answer: (id: number, ok: boolean) => void;
}) {
  const messageId = useId();
  const cancel = () => {
    if (pending) answer(pending.id, false);
  };
  // A destructive question opens on "Abbrechen" (WAI-ARIA APG alertdialog),
  // so a repeated or held Enter on the trigger cannot confirm it. A neutral
  // one opens on the confirm button, like the native dialog did.
  const danger = pending?.tone !== "neutral";
  return (
    <Modal
      open={pending !== null}
      onClose={cancel}
      title={pending?.title ?? ""}
      role="alertdialog"
      describedBy={pending?.message ? messageId : undefined}
    >
      {pending ? (
        <form
          // A replacing question remounts, so its own initial focus applies.
          key={pending.id}
          className="flex flex-col gap-5"
          onSubmit={(event) => {
            event.preventDefault();
            answer(pending.id, true);
          }}
        >
          {pending.message ? (
            <p
              id={messageId}
              className="text-sm text-neutral-600 dark:text-neutral-400"
            >
              {pending.message}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <GhostButton onClick={cancel} initialFocus={danger}>
              Abbrechen
            </GhostButton>
            <button
              type="submit"
              {...dialogInitialFocus(!danger)}
              className={`rounded-lg px-4 py-2 text-sm font-medium transition hover:brightness-110 ${
                danger ? "gel-confirm-danger" : "lr-submit"
              }`}
            >
              {pending.confirmLabel}
            </button>
          </div>
        </form>
      ) : null}
    </Modal>
  );
}
