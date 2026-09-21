import { useEffect, useRef, type ReactNode } from "react";

type ModalProps = {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  wide?: boolean;
};

/**
 * Native `<dialog>` so focus trapping, Escape and the backdrop come from the
 * browser. Closes on Escape and on a click outside the panel.
 */
export function Modal({ open, onClose, title, children, wide }: ModalProps) {
  const ref = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);

  return (
    <dialog
      ref={ref}
      aria-labelledby="modal-title"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      className={[
        "m-auto w-[calc(100%-2rem)] rounded-2xl border border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 p-0 text-neutral-900 dark:text-neutral-100 shadow-xl backdrop:bg-neutral-900/40 dark:backdrop:bg-black/60",
        wide ? "max-w-lg" : "max-w-md",
      ].join(" ")}
    >
      {open ? (
        <div className="flex flex-col gap-5 p-6">
          <h2 id="modal-title" className="text-lg font-semibold tracking-tight">
            {title}
          </h2>
          {children}
        </div>
      ) : null}
    </dialog>
  );
}

/** Secondary button for dialogs ("Abbrechen" and friends). */
export function GhostButton({
  children,
  onClick,
  disabled,
  tone = "neutral",
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  tone?: "neutral" | "danger";
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled}
      className={[
        "rounded-lg px-3 py-2 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-60",
        tone === "danger"
          ? "text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-950"
          : "text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800",
      ].join(" ")}
    >
      {children}
    </button>
  );
}
