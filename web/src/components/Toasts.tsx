import { CloseIcon, WarningIcon } from "./Icons.tsx";
import { useToasts, type Toast } from "./toasts.ts";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import "./ui/ui.css";

/**
 * Both live regions are always in the page, empty or not: a region that
 * appears together with its first message is not announced by most screen
 * readers. Notices are polite, errors interrupt.
 */
export function Toasts() {
  return <ToastRegions toasts={useToasts((state) => state.toasts)} />;
}

export function ToastRegions({ toasts }: { toasts: Toast[] }) {
  return (
    <div className="gel-toasts">
      <div role="status" className="gel-toasts__region">
        {toasts
          .filter((toast) => toast.tone === "info")
          .map((toast) => (
            <ToastCard key={toast.id} toast={toast} />
          ))}
      </div>
      <div role="alert" className="gel-toasts__region">
        {toasts
          .filter((toast) => toast.tone === "error")
          .map((toast) => (
            <ToastCard key={toast.id} toast={toast} />
          ))}
      </div>
    </div>
  );
}

function ToastCard({ toast }: { toast: Toast }) {
  const dismiss = useToasts((state) => state.dismiss);
  const hold = useToasts((state) => state.hold);
  const release = useToasts((state) => state.release);
  return (
    <div
      className={`ui-toast ui-toast--${toast.tone}`}
      onPointerEnter={() => hold(toast.id, "hover")}
      onPointerLeave={() => release(toast.id, "hover")}
      onFocus={() => hold(toast.id, "focus")}
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
            dismiss(toast.id);
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
        onClick={() => dismiss(toast.id)}
      />
    </div>
  );
}
