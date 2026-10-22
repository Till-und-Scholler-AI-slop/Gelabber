import { Fragment, useId } from "react";

import { useConfirm, type ConfirmRequest } from "./confirm.ts";
import { Modal } from "./Modal.tsx";
import { Button } from "./ui/Button.tsx";

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
  const formId = useId();
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
      footer={
        pending ? (
          // A replacing question remounts, so its own initial focus applies.
          <Fragment key={pending.id}>
            <Button variant="ghost" onClick={cancel} initialFocus={danger}>
              Abbrechen
            </Button>
            <Button
              type="submit"
              form={formId}
              variant={danger ? "danger" : "primary"}
              initialFocus={!danger}
            >
              {pending.confirmLabel}
            </Button>
          </Fragment>
        ) : null
      }
    >
      {pending ? (
        <form
          key={pending.id}
          id={formId}
          onSubmit={(event) => {
            event.preventDefault();
            answer(pending.id, true);
          }}
        >
          {pending.message ? (
            <p id={messageId} className="text-lr-muted">
              {pending.message}
            </p>
          ) : null}
        </form>
      ) : null}
    </Modal>
  );
}
