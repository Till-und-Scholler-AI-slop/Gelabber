import { useNavigate } from "@tanstack/react-router";

import { useMessageToasts, type MessageToast } from "../messages/toasts.ts";
import { Avatar } from "./Avatar.tsx";
import { CloseIcon } from "./Icons.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { usePresenceList } from "./ui/usePresence.ts";
import "./ui/ui.css";

const toastKey = (toast: MessageToast) => toast.id;

export function MessageToasts() {
  const toasts = useMessageToasts((state) => state.toasts);
  const dismiss = useMessageToasts((state) => state.dismiss);
  const hold = useMessageToasts((state) => state.hold);
  const release = useMessageToasts((state) => state.release);
  const navigate = useNavigate();
  // One that is opened, closed or runs out slides away before it goes.
  const { entries, ref } = usePresenceList(toasts, toastKey);

  return (
    <div aria-live="polite" className="gel-message-toasts">
      {entries.map(({ key, item: toast, state }) => {
        const where =
          toast.count > 1
            ? `${toast.channelLabel} · ${toast.count} neu`
            : toast.channelLabel;
        return (
          <div
            key={key}
            ref={ref(key)}
            data-state={state}
            inert={state === "closed"}
            className="ui-message-toast"
            onPointerEnter={() => hold(toast.id, "hover")}
            onPointerLeave={() => release(toast.id, "hover")}
            onFocus={() => hold(toast.id, "focus")}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget))
                release(toast.id, "focus");
            }}
          >
            <Avatar
              name={toast.author}
              url={toast.authorAvatar ?? null}
              decorative
            />
            <button
              type="button"
              className="ui-message-toast__open"
              aria-label={`Öffnen: ${where}, ${toast.author}${
                toast.mentioned ? " hat dich erwähnt" : ""
              }: ${toast.preview}`}
              onClick={() => {
                dismiss(toast.id);
                if (toast.dm) {
                  void navigate({
                    to: "/d/$channelId",
                    params: { channelId: toast.channelId },
                  });
                  return;
                }
                void navigate({
                  to: "/s/$serverId/c/$channelId",
                  params: {
                    serverId: toast.serverId,
                    channelId: toast.channelId,
                  },
                });
              }}
            >
              <span className="ui-message-toast__where">{where}</span>
              <span className="ui-message-toast__who">
                {toast.author}
                {toast.mentioned ? <small> hat dich erwähnt</small> : null}
              </span>
              <span className="ui-message-toast__preview">{toast.preview}</span>
            </button>
            <IconButton
              size="xs"
              label="Schließen"
              className="ui-message-toast__close"
              icon={<CloseIcon size={14} />}
              onClick={() => dismiss(toast.id)}
            />
          </div>
        );
      })}
    </div>
  );
}
