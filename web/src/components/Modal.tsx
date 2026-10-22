import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  type ReactNode,
} from "react";

import { CloseIcon } from "./Icons.tsx";
import {
  closesOnBackdrop,
  focusLeftWith,
  focusOrigin,
  markLeaving,
  requestClose,
  restoreFocus,
} from "./modalClose.ts";
import { IconButton } from "./ui/IconButton.tsx";
import { useLastOpen, usePresence } from "./ui/usePresence.ts";
import "./ui/ui.css";

export { GhostButton } from "./ui/Button.tsx";

type ModalProps = {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  wide?: boolean;
  /** `alertdialog` for questions that interrupt, like a confirm. */
  role?: "dialog" | "alertdialog";
  /** Id of the element that explains the dialog. */
  describedBy?: string;
  /** Actions pinned under the scrolling body, in thumb reach on phones. */
  footer?: ReactNode;
  /**
   * Unsaved input: a backdrop click is ignored, and Esc or "Schließen"
   * asks before discarding it.
   */
  dirty?: boolean;
};

/**
 * Native `<dialog>` so focus trapping, Escape and the backdrop come from the
 * browser. The title stays put while the body scrolls, and "Schließen" is
 * always there. Under 640px it is a bottom sheet. Closed, it stays modal and
 * inert for its exit, still showing its last content, then hands focus back.
 */
export function Modal({
  open,
  onClose,
  title,
  children,
  wide,
  role,
  describedBy,
  footer,
  dirty = false,
}: ModalProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const titleId = useId();
  const latest = useRef({ open, onClose });
  useLayoutEffect(() => {
    latest.current = { open, onClose };
  });
  const close = () => void requestClose({ dirty, onClose });
  const { mounted, state } = usePresence(open, ref);
  const [shownTitle, shownChildren, shownFooter] = useLastOpen(open, [
    title,
    children,
    footer,
  ] as const);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      openerRef.current = focusOrigin(document.activeElement);
      dialog.showModal();
    }
    if (!open && dialog.open) {
      // Still leaving: a dialog opened meanwhile takes over the opener.
      if (mounted) return markLeaving(dialog, openerRef.current);
      dialog.close();
      if (focusLeftWith(dialog, document.activeElement))
        restoreFocus(openerRef.current);
      openerRef.current = null;
    }
  }, [open, mounted]);

  useEffect(() => {
    const dialog = ref.current;
    return () => {
      if (dialog?.open) dialog.close();
    };
  }, []);

  return (
    <dialog
      ref={ref}
      role={role === "alertdialog" ? role : undefined}
      aria-labelledby={titleId}
      aria-describedby={describedBy}
      data-state={state}
      inert={!open && mounted}
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        if (open) close();
      }}
      onClose={(event) => {
        // A browser may close a dialog without a cancel event (Chrome does
        // for a repeated Esc); keep the owner's state in step. The event is
        // queued, so it can arrive after the dialog was shown again (one
        // mounted open closes and reopens at once under StrictMode): a
        // dialog that is open by then was not closed.
        if (event.currentTarget.open) return;
        if (latest.current.open) latest.current.onClose();
      }}
      onClick={(event) => {
        if (
          open &&
          event.target === event.currentTarget &&
          closesOnBackdrop(
            event,
            event.currentTarget.getBoundingClientRect(),
            dirty,
          )
        )
          onClose();
      }}
      className={wide ? "gel-modal gel-modal--wide" : "gel-modal"}
    >
      {mounted ? (
        <>
          <header className="gel-modal__header">
            <h2 id={titleId}>{shownTitle}</h2>
          </header>
          <div className="gel-modal__body">{shownChildren}</div>
          {shownFooter ? (
            <footer className="gel-modal__footer">{shownFooter}</footer>
          ) : null}
          {/* Last in the DOM, so a dialog still opens on its first field;
              drawn in the header's corner. */}
          <IconButton
            className="gel-modal__close"
            label="Schließen"
            icon={<CloseIcon size={18} />}
            onClick={close}
          />
        </>
      ) : null}
    </dialog>
  );
}
