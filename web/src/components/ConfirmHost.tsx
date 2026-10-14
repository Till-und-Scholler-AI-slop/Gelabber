import { useConfirm } from "./confirm.ts";
import { GhostButton, Modal } from "./Modal.tsx";

export function ConfirmHost() {
  const pending = useConfirm((state) => state.pending);
  const answer = useConfirm((state) => state.answer);
  const cancel = () => {
    if (pending) answer(pending.id, false);
  };
  return (
    <Modal
      open={pending !== null}
      onClose={cancel}
      title={pending?.title ?? ""}
    >
      {pending ? (
        <form
          className="flex flex-col gap-5"
          onSubmit={(event) => {
            event.preventDefault();
            answer(pending.id, true);
          }}
        >
          {pending.message ? (
            <p className="text-sm text-neutral-600 dark:text-neutral-400">
              {pending.message}
            </p>
          ) : null}
          <div className="flex justify-end gap-2">
            <GhostButton onClick={cancel}>Abbrechen</GhostButton>
            <button
              type="submit"
              // Like the native dialog: Enter confirms right away. The
              // attribute, not React's autoFocus, is what showModal() reads.
              ref={(button) => button?.setAttribute("autofocus", "")}
              className={`rounded-lg px-4 py-2 text-sm font-medium transition hover:brightness-110 ${
                pending.tone === "neutral" ? "lr-submit" : "gel-confirm-danger"
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
