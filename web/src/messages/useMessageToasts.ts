import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { serverNow } from "../api/client.ts";
import { scopeGeneration, stampHolds } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { dmKeys } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import { serverKeys } from "../servers/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { getGateway } from "../ws/client.ts";
import type { ChatEvent } from "../ws/protocol.ts";
import {
  closeMessageNotifications,
  showMessageNotification,
} from "../pwa/notifications.ts";
import {
  conversationPath,
  createNotificationDedupe,
  isDmTopic,
  messageNotificationDecision,
  previewText,
} from "./notify.ts";
import { useMessageToasts } from "./toasts.ts";
import type { Message } from "./types.ts";
import { asAttachmentList } from "./types.ts";

function asCreated(delta: unknown): Message | null {
  if (delta === null || typeof delta !== "object") return null;
  const value = delta as Partial<Message>;
  if (
    typeof value.id !== "string" ||
    typeof value.channel_id !== "string" ||
    typeof value.content !== "string" ||
    !value.author ||
    typeof value.author.id !== "string" ||
    typeof value.author.name !== "string"
  ) {
    return null;
  }
  return {
    id: value.id,
    channel_id: value.channel_id,
    author: value.author,
    content: value.content,
    created_at: value.created_at ?? new Date().toISOString(),
    edited_at: value.edited_at ?? null,
    attachments: asAttachmentList(value.attachments),
  };
}

function channelLabel(
  client: ReturnType<typeof useQueryClient>,
  userId: string,
  generation: number,
  event: ChatEvent,
  dm: boolean,
): string {
  if (!event.c) return dm ? "DM" : "Kanal";
  if (dm) {
    const list = client.getQueryData<DirectMessage[]>(
      dmKeys.list(userId, generation),
    );
    return list?.find((row) => row.id === event.c)?.peer.name ?? "DM";
  }
  const server = client.getQueryData<ServerDetail>(
    serverKeys.detail(userId, generation, event.s),
  );
  const channel = server?.channels.find((row) => row.id === event.c);
  return channel ? `#${channel.name}` : "Kanal";
}

/** Toast + optional desktop notification for creates in another chat. */
export function useMessageToastsBridge(): void {
  const client = useQueryClient();
  const navigate = useNavigate();
  const me = useSession((s) => s.user?.id);
  const viewingChannelId = useParams({ strict: false }).channelId;
  useEffect(() => closeMessageNotifications, [me]);
  const deliveries = useRef<{
    userId: string | undefined;
    generation: number;
    first: ReturnType<typeof createNotificationDedupe>;
    desktopAt: Map<string, number>;
  } | null>(null);

  useEffect(() => {
    const userId = me;
    const generation = scopeGeneration();
    if (
      deliveries.current?.userId !== userId ||
      deliveries.current?.generation !== generation
    ) {
      deliveries.current = {
        userId,
        generation,
        first: createNotificationDedupe(),
        desktopAt: new Map(),
      };
    }
    const firstDelivery = deliveries.current.first;
    return getGateway().onEvent((event) => {
      if (!userId || !stampHolds({ userId, generation })) return;
      if (event.t !== "c" || !event.c) return;
      const message = asCreated(event.d);
      if (!message) return;
      if (!firstDelivery(event.c, message.id)) return;
      const settings = useMediaSettings.getState();
      const decision = messageNotificationDecision({
        toastEnabled: settings.messageToasts,
        desktopEnabled: settings.desktopNotify,
        hidden: typeof document !== "undefined" && document.hidden,
        type: event.t,
        own: message.author.id === userId,
        channelId: event.c,
        viewingChannelId,
      });
      if (!decision.toast && !decision.desktop) return;
      const dm = isDmTopic(event.s, event.c);
      const label = channelLabel(client, userId, generation, event, dm);
      const preview = previewText(
        message.content,
        asAttachmentList(message.attachments).length > 0,
      );
      if (decision.toast)
        useMessageToasts.getState().push({
          channelId: event.c,
          serverId: event.s,
          dm,
          channelLabel: label,
          author: message.author.name,
          preview,
        });
      const now = Date.now();
      const desktopAt = deliveries.current!.desktopAt;
      // Replays older than the live delivery window do not generate a burst.
      // Age is measured on the server clock: a fast local clock must not
      // suppress every live notification.
      if (
        decision.desktop &&
        serverNow() - Date.parse(message.created_at) < 30_000 &&
        now - (desktopAt.get(event.c) ?? 0) >= 5_000
      ) {
        desktopAt.set(event.c, now);
        const channelId = event.c;
        void showMessageNotification(
          {
            title: `${message.author.name} · ${label}`,
            body: preview,
            tag: `gelabber:${userId}:${channelId}`,
            path: conversationPath(dm, event.s, channelId),
            user: userId,
          },
          () => {
            if (!stampHolds({ userId, generation })) return;
            window.focus();
            if (dm) {
              void navigate({ to: "/d/$channelId", params: { channelId } });
            } else {
              void navigate({
                to: "/s/$serverId/c/$channelId",
                params: { serverId: event.s, channelId },
              });
            }
          },
        );
      }
    });
  }, [client, me, navigate, viewingChannelId]);
}
