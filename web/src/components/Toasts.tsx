import { useLayoutEffect, useRef, type RefObject } from "react";

import { CloseIcon, WarningIcon } from "./Icons.tsx";
import { restoreFocus } from "./modalClose.ts";
import { useToasts, type Toast } from "./toasts.ts";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import type { PresenceState } from "./ui/presence.ts";
import { usePresenceList } from "./ui/usePresence.ts";
import "./ui/ui.css";

const toastKey = (toast: Toast) => toast.id;

/**
 * Both live regions are always in the page, empty or not: a region that
 * appears together with its first message is not announced by most screen
 * readers. Notices are polite, errors interrupt.
 */
export function Toasts() {
  return <ToastRegions toasts={useToasts((state) => state.toasts)} />;
}

export function ToastRegions({ toasts }: { toasts: Toast[] }) {
  // A dismissed or expired toast fades out before it goes.
  const { entries, ref } = usePresenceList(toasts, toastKey);
  const cards = (tone: Toast["tone"]) =>
    entries
      .filter((entry) => entry.item.tone === tone)
      .map((entry) => (
        <ToastCard
          key={entry.key}
          toast={entry.item}
          state={entry.state}
          cardRef={ref(entry.key)}
        />
      ));
  const stack = useRef<HTMLDivElement>(null);
  useToastSpace(stack);
  return (
    <div ref={stack} className="gel-toasts">
      <div role="status" className="gel-toasts__region">
        {cards("info")}
      </div>
      <div role="alert" className="gel-toasts__region">
        {cards("error")}
      </div>
    </div>
  );
}

/**
 * How tall the stack of toasts is, as `--lr-toast-space` on the page, 0 with
 * none. In a conversation the stack sits above the composer, and the message
 * list makes this much room for it: otherwise a toast covers the newest
 * message and its actions (#354). A leaving toast keeps its room until it
 * has faded out.
 */
function useToastSpace(stack: RefObject<HTMLDivElement | null>) {
  useLayoutEffect(() => {
    const element = stack.current;
    if (!element) return;
    const update = () => {
      let top = Infinity;
      let bottom = -Infinity;
      // Layout boxes: a card's rise and fade (translate, scale) do not count.
      for (const card of element.querySelectorAll<HTMLElement>(".ui-toast")) {
        top = Math.min(top, card.offsetTop);
        bottom = Math.max(bottom, card.offsetTop + card.offsetHeight);
      }
      const height = bottom > top ? bottom - top : 0;
      document.documentElement.style.setProperty(
        "--lr-toast-space",
        `${Math.ceil(height)}px`,
      );
    };
    // A toast that comes or goes, a message that wraps to another line, or
    // another text size: each changes the stack's own height.
    const observer = new ResizeObserver(update);
    observer.observe(element);
    return () => {
      observer.disconnect();
      document.documentElement.style.removeProperty("--lr-toast-space");
    };
  }, [stack]);
}

function ToastCard({
  toast,
  state,
  cardRef,
}: {
  toast: Toast;
  state: PresenceState;
  cardRef: (element: Element | null) => void;
}) {
  const dismiss = useToasts((state) => state.dismiss);
  const hold = useToasts((state) => state.hold);
  const release = useToasts((state) => state.release);
  // Where focus was before it came into the toast: it goes back there when
  // the toast closes under it, instead of falling to the page.
  const cameFrom = useRef<HTMLElement | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const close = () => {
    // Unless the action took focus somewhere itself.
    const focusInside = card.current?.contains(document.activeElement);
    dismiss(toast.id);
    if (focusInside) restoreFocus(cameFrom.current);
  };
  return (
    <div
      ref={(element) => {
        card.current = element;
        cardRef(element);
      }}
      data-state={state}
      inert={state === "closed"}
      className={`ui-toast ui-toast--${toast.tone}`}
      onPointerEnter={() => hold(toast.id, "hover")}
      onPointerLeave={() => release(toast.id, "hover")}
      onFocus={(event) => {
        const from = event.relatedTarget;
        if (from instanceof HTMLElement && !event.currentTarget.contains(from))
          cameFrom.current = from;
        hold(toast.id, "focus");
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget))
          release(toast.id, "focus");
      }}
    >
      {toast.tone === "error" ? (
        <WarningIcon size={16} className="ui-toast__icon" />
      ) : null}
      <p className="ui-toast__message">{toast.message}</p>
      {toast.action ? (
        <Button
          variant="ghost"
          size="sm"
          className="ui-toast__action"
          onClick={() => {
            toast.action?.onClick();
            close();
          }}
        >
          {toast.action.label}
        </Button>
      ) : null}
      <IconButton
        size="sm"
        label="Schließen"
        className="ui-toast__close"
        icon={<CloseIcon size={16} />}
        onClick={close}
      />
    </div>
  );
}
