import { useNavigate } from "@tanstack/react-router";

import { useMessageToasts, type MessageToast } from "../messages/toasts.ts";
import { useVoice } from "../voice/session.ts";
import { currentCall, joinToWatch, watchLabel } from "../voice/watchLive.ts";
import { Avatar } from "./Avatar.tsx";
import { CloseIcon } from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { usePresenceList } from "./ui/usePresence.ts";
import { T, useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { common } from "../i18n/catalogs/common.ts";
import "./ui/ui.css";

const toastKey = (toast: MessageToast) => toast.id;

export function MessageToasts() {
  const t = useT(chat);
  const tc = useT(common);
  const toasts = useMessageToasts((state) => state.toasts);
  const dismiss = useMessageToasts((state) => state.dismiss);
  const hold = useMessageToasts((state) => state.hold);
  const release = useMessageToasts((state) => state.release);
  const navigate = useNavigate();
  const call = useVoice(currentCall);
  // One that is opened, closed or runs out slides away before it goes.
  const { entries, ref } = usePresenceList(toasts, toastKey);

  return (
    <div aria-live="polite" className="gel-message-toasts">
      {entries.map(({ key, item: toast, state }) => {
        const where =
          toast.count > 1
            ? t("toast.newCount", {
                where: toast.channelLabel,
                count: toast.count,
              })
            : toast.channelLabel;
        const live = toast.live;
        const open = () =>
          void navigate({
            to: "/s/$serverId/c/$channelId",
            params: {
              serverId: toast.serverId,
              channelId: toast.channelId,
            },
          });
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
              aria-label={
                live
                  ? t("toast.openLiveLabel", { where, preview: toast.preview })
                  : t(
                      toast.mentioned
                        ? "toast.openMentionedLabel"
                        : "toast.openLabel",
                      { where, author: toast.author, preview: toast.preview },
                    )
              }
              onClick={() => {
                dismiss(toast.id);
                if (toast.dm) {
                  void navigate({
                    to: "/d/$channelId",
                    params: { channelId: toast.channelId },
                  });
                  return;
                }
                open();
              }}
            >
              <span className="ui-message-toast__where">{where}</span>
              <span className="ui-message-toast__who">
                {toast.author}
                {toast.mentioned ? (
                  <small> {t("toast.mentionedYou")}</small>
                ) : null}
              </span>
              {live ? (
                // The name stands once, above: here only where.
                <span className="ui-message-toast__preview ui-message-toast__live">
                  <T
                    catalog={chat}
                    k="toast.live"
                    params={{ voice: live.voice }}
                    tags={{ b: (text) => <b>{text}</b> }}
                  />
                </span>
              ) : (
                <span className="ui-message-toast__preview">
                  {toast.preview}
                </span>
              )}
            </button>
            {live ? (
              <Button
                size="sm"
                className="ui-message-toast__action"
                onClick={() => {
                  dismiss(toast.id);
                  open();
                  joinToWatch({
                    serverId: toast.serverId,
                    channelId: toast.channelId,
                    channelName: live.voice,
                    people: [live.userId],
                  });
                }}
              >
                {watchLabel(call, toast.serverId, toast.channelId)}
              </Button>
            ) : null}
            <IconButton
              size="xs"
              label={tc("close")}
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
