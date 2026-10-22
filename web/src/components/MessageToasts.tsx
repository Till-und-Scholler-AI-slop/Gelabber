import { useNavigate } from "@tanstack/react-router";

import { useMessageToasts } from "../messages/toasts.ts";
import { Avatar } from "./Avatar.tsx";
import { CloseIcon } from "./Icons.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import "./ui/ui.css";

export function MessageToasts() {
  const toasts = useMessageToasts((state) => state.toasts);
  const dismiss = useMessageToasts((state) => state.dismiss);
  const navigate = useNavigate();

  return (
    <div aria-live="polite" className="gel-message-toasts">
      {toasts.map((toast) => {
        const where =
          toast.count > 1
            ? `${toast.channelLabel} · ${toast.count} neu`
            : toast.channelLabel;
        return (
          <div key={toast.id} className="ui-message-toast">
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
