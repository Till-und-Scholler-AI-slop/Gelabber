import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

import { dialogInitialFocus } from "../dialogFocus.ts";
import { outsideBox } from "../modalClose.ts";
import { useMediaQuery } from "../useMediaQuery.ts";
import {
  POPOVER_SHEET_QUERY,
  popoverPosition,
  type PopoverPlacement,
} from "./popoverPosition.ts";
import "./ui.css";

export type { PopoverPlacement };

const FOCUSABLE =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

function focusInside(container: HTMLElement) {
  const target =
    container.querySelector<HTMLElement>("[autofocus]") ??
    container.querySelector<HTMLElement>(FOCUSABLE) ??
    container;
  target.focus({ preventScroll: true });
}

type PopoverProps = {
  open: boolean;
  onClose: () => void;
  /** The control that opens it: the popover sits next to it, and focus goes back to it. */
  anchor: RefObject<HTMLElement | null>;
  /** Names the popover; the bottom sheet also shows it as its title. */
  label: string;
  placement?: PopoverPlacement;
  /** `menu` when the children are `menuitem` entries. */
  role?: "dialog" | "menu";
  children: ReactNode;
  className?: string;
};

/**
 * A small panel attached to the control that opened it. Esc, a press
 * outside and tabbing away close it; focus starts inside and returns to the
 * opener. Under 640px or on a touch screen it is a bottom sheet.
 */
export function Popover(props: PopoverProps) {
  const sheet = useMediaQuery(POPOVER_SHEET_QUERY);
  if (!props.open) return null;
  return sheet ? <PopoverSheet {...props} /> : <AnchoredPopover {...props} />;
}

function useLatest<T>(value: T) {
  const ref = useRef(value);
  useLayoutEffect(() => {
    ref.current = value;
  });
  return ref;
}

/** Back to the opener when focus was in the panel, or was lost with it. */
function returnFocus(panel: HTMLElement | null, anchor: HTMLElement | null) {
  const active = document.activeElement;
  if (
    anchor?.isConnected &&
    (active === null || active === document.body || panel?.contains(active))
  )
    anchor.focus({ preventScroll: true });
}

function AnchoredPopover({
  onClose,
  anchor,
  label,
  placement = "bottom-start",
  role = "dialog",
  children,
  className,
}: PopoverProps) {
  const panel = useRef<HTMLDivElement>(null);
  const close = useLatest(onClose);
  // Rendered into the dialog it is used in, if any: outside a modal dialog
  // the page is inert. A marker where the popover is placed finds it.
  const [host, setHost] = useState<Element | null>(null);
  const marker = useCallback((element: HTMLElement | null) => {
    if (element) setHost(element.closest("dialog") ?? document.body);
  }, []);

  useLayoutEffect(() => {
    const element = panel.current;
    const opener = anchor.current;
    if (!element || !opener) return;
    const place = () => {
      const { top, left } = popoverPosition(
        opener.getBoundingClientRect(),
        { width: element.offsetWidth, height: element.offsetHeight },
        { width: window.innerWidth, height: window.innerHeight },
        placement,
      );
      element.style.top = `${top}px`;
      element.style.left = `${left}px`;
    };
    place();
    focusInside(element);
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!element.contains(target) && !opener.contains(target))
        close.current();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // Handled here: an enclosing dialog must not close on the same Esc.
      event.preventDefault();
      event.stopPropagation();
      opener.focus({ preventScroll: true });
      close.current();
    };
    const onFocusOut = (event: FocusEvent) => {
      const next = event.relatedTarget as Node | null;
      if (next && !element.contains(next) && !opener.contains(next))
        close.current();
    };
    const resize = new ResizeObserver(place);
    resize.observe(element);
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    element.addEventListener("focusout", onFocusOut);
    return () => {
      resize.disconnect();
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
      element.removeEventListener("focusout", onFocusOut);
      returnFocus(element, opener);
    };
  }, [anchor, close, placement, host]);

  return (
    <>
      <span ref={marker} hidden />
      {host
        ? createPortal(
            <div
              ref={panel}
              role={role}
              aria-label={label}
              tabIndex={-1}
              className={["ui-popover", className].filter(Boolean).join(" ")}
            >
              {children}
            </div>,
            host,
          )
        : null}
    </>
  );
}

function PopoverSheet({
  onClose,
  anchor,
  label,
  role = "dialog",
  children,
  className,
}: PopoverProps) {
  const dialog = useRef<HTMLDialogElement>(null);
  useLayoutEffect(() => {
    const element = dialog.current;
    const opener = anchor.current;
    if (!element) return;
    element.showModal();
    return () => {
      if (element.open) element.close();
      returnFocus(element, opener);
    };
  }, [anchor]);
  return createPortal(
    <dialog
      ref={dialog}
      aria-label={label}
      className={["ui-sheet", className].filter(Boolean).join(" ")}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (
          event.target === event.currentTarget &&
          outsideBox(event, event.currentTarget.getBoundingClientRect())
        )
          onClose();
      }}
    >
      <div className="ui-sheet__grip" aria-hidden="true" />
      {/* Focus starts on the title, not on the first entry: on a phone that
          would open the keyboard for a text field or suggest a choice
          nobody made. */}
      <h2
        className="ui-sheet__title"
        tabIndex={-1}
        {...dialogInitialFocus(true)}
      >
        {label}
      </h2>
      <div
        role={role === "menu" ? "menu" : undefined}
        aria-label={role === "menu" ? label : undefined}
      >
        {children}
      </div>
    </dialog>,
    document.body,
  );
}
