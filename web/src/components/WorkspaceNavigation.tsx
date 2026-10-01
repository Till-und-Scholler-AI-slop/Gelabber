import { useEffect, useId, useRef, useState, type ReactNode } from "react";

import { ChevronIcon } from "./Icons.tsx";

/** One content tree: ordinary desktop navigation, a native modal on mobile. */
export function WorkspaceDrawer({
  open,
  onClose,
  title,
  breakpoint,
  className,
  id,
  children,
  closeOnNavigate = false,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  breakpoint: number;
  className: string;
  id: string;
  children: ReactNode;
  closeOnNavigate?: boolean;
}) {
  const query = `(max-width: ${breakpoint}px)`;
  const [mobile, setMobile] = useState(
    () => typeof window !== "undefined" && window.matchMedia(query).matches,
  );
  const dialogRef = useRef<HTMLDialogElement>(null);
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const modalRef = useRef(false);
  const onCloseRef = useRef(onClose);
  const headingId = useId();

  useEffect(() => {
    onCloseRef.current = onClose;
  }, [onClose]);

  useEffect(() => {
    const media = window.matchMedia(query);
    const update = () => setMobile(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, [query]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const restoreFocus = () => {
      const opener = openerRef.current;
      if (opener?.isConnected && opener.getClientRects().length > 0)
        opener.focus({ preventScroll: true });
      openerRef.current = null;
    };

    if (!mobile) {
      if (modalRef.current) {
        dialog.close();
        modalRef.current = false;
        restoreFocus();
        onCloseRef.current();
      }
      // Setting the non-modal attribute avoids show() stealing focus on load.
      if (!dialog.open) dialog.setAttribute("open", "");
      return;
    }
    if (dialog.open && !modalRef.current) dialog.close();
    if (open && !dialog.open) {
      openerRef.current =
        document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
      dialog.showModal();
      modalRef.current = true;
      closeButtonRef.current?.focus();
    } else if (!open && modalRef.current) {
      dialog.close();
      modalRef.current = false;
      restoreFocus();
    }
  }, [mobile, open]);

  useEffect(() => {
    const dialog = dialogRef.current;
    return () => {
      if (dialog?.open) dialog.close();
      const opener = openerRef.current;
      if (opener?.isConnected && opener.getClientRects().length > 0)
        opener.focus({ preventScroll: true });
    };
  }, []);

  return (
    <dialog
      ref={dialogRef}
      id={id}
      className={`workspace-drawer ${className}`}
      role={mobile ? undefined : "presentation"}
      aria-modal={mobile && open ? true : undefined}
      aria-labelledby={mobile ? headingId : undefined}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (!mobile) return;
        if (
          closeOnNavigate &&
          event.target instanceof Element &&
          event.target.closest("a[href]") &&
          event.target.closest("dialog") === event.currentTarget
        ) {
          onClose();
          return;
        }
        if (event.target !== event.currentTarget) return;
        const bounds = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < bounds.left ||
          event.clientX > bounds.right ||
          event.clientY < bounds.top ||
          event.clientY > bounds.bottom
        )
          onClose();
      }}
    >
      <header className="workspace-drawer-header">
        <h2 id={headingId}>{title}</h2>
        <button
          ref={closeButtonRef}
          type="button"
          className="workspace-drawer-close"
          onClick={onClose}
          aria-label={`${title} schließen`}
        >
          <ChevronIcon size={18} />
          Schließen
        </button>
      </header>
      <div className="workspace-drawer-body">{children}</div>
    </dialog>
  );
}
