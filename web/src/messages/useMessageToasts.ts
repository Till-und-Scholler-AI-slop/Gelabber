import { useQueryClient } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { useEffect } from "react";

import { scopeGeneration, stampHolds } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { dmKeys } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import { serverKeys } from "../servers/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { getGateway } from "../ws/client.ts";
import type { ChatEvent } from "../ws/protocol.ts";
import { isDmTopic, previewText, shouldToastMessage } from "./notify.ts";
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

function maybeDesktopNotify(title: string, body: string): void {
  if (typeof document === "undefined" || !document.hidden) return;
  if (!useMediaSettings.getState().desktopNotify) return;
  const Notify = (
    globalThis as unknown as {
      Notification?: {
        permission: string;
        new (title: string, opts?: { body: string; silent?: boolean }): unknown;
      };
    }
  ).Notification;
  if (!Notify || Notify.permission !== "granted") return;
  try {
    new Notify(title, { body, silent: true });
  } catch {
    // permission revoked mid-flight
  }
}

/** Toast + optional desktop notification for creates in another chat. */
export function useMessageToastsBridge(): void {
  const client = useQueryClient();
  const me = useSession((s) => s.user?.id);
  const viewingChannelId = useParams({ strict: false }).channelId;

  useEffect(() => {
    const userId = me;
    const generation = scopeGeneration();
    return getGateway().onEvent((event) => {
      if (!userId || !stampHolds({ userId, generation })) return;
      if (event.t !== "c" || !event.c) return;
      const message = asCreated(event.d);
      if (!message) return;
      const enabled = useMediaSettings.getState().messageToasts;
      if (
        !shouldToastMessage({
          enabled,
          type: event.t,
          own: message.author.id === userId,
          channelId: event.c,
          viewingChannelId,
        })
      ) {
        return;
      }
      const dm = isDmTopic(event.s, event.c);
      const label = channelLabel(client, userId, generation, event, dm);
      const preview = previewText(
        message.content,
        asAttachmentList(message.attachments).length > 0,
      );
      useMessageToasts.getState().push({
        channelId: event.c,
        serverId: event.s,
        dm,
        channelLabel: label,
        author: message.author.name,
        preview,
      });
      maybeDesktopNotify(`${message.author.name} · ${label}`, preview);
    });
  }, [client, me, viewingChannelId]);
}
