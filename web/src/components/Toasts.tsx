import { useRef } from "react";

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
  return (
    <div className="gel-toasts">
      <div role="status" className="gel-toasts__region">
        {cards("info")}
      </div>
      <div role="alert" className="gel-toasts__region">
        {cards("error")}
      </div>
    </div>
  );
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
