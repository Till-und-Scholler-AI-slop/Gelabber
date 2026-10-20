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
import { playMessageSound } from "../voice/callSounds.ts";
import { useMediaSettings } from "../voice/settings.ts";
import { useVoice } from "../voice/session.ts";
import { useInstallation } from "../pwa/install.ts";
import { getGateway } from "../ws/client.ts";
import type { ChatEvent } from "../ws/protocol.ts";
import {
  closeMessageNotifications,
  showMessageNotification,
  silenceWhileViewing,
} from "../pwa/notifications.ts";
import {
  conversationPath,
  createNotificationDedupe,
  createNotificationPacing,
  isDmTopic,
  messageNotificationDecision,
  previewText,
} from "./notify.ts";
import { focusDesktopWindow, requestDesktopAttention } from "./desktopShell.ts";
import {
  mentionsMe,
  readableMentions,
  type MentionMember,
} from "./mentions.ts";
import { wantsNotification } from "./notifyLevel.ts";
import {
  messageNotificationText,
  useNotificationPrivacy,
} from "./notificationPrivacy.ts";
import {
  conversationSetting,
  settingsKey,
  useNotificationSettings,
  type NotificationSetting,
} from "./notificationSettings.ts";
import { readKey, type ReadState } from "./readState.ts";
import { useMessageToasts } from "./toasts.ts";
import type { Message } from "./types.ts";
import { asAttachmentList, asGif, asMentionList } from "./types.ts";

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
    gif: asGif(value.gif),
    mentions: asMentionList(value.mentions),
    mention_everyone: value.mention_everyone === true,
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

/** Names for the mention tokens of a preview, from what is cached. */
function conversationMembers(
  client: ReturnType<typeof useQueryClient>,
  userId: string,
  generation: number,
  event: ChatEvent,
  dm: boolean,
): MentionMember[] {
  const me = useSession.getState().user;
  const own = me ? [{ id: me.id, name: me.name }] : [];
  if (dm) {
    const peer = client
      .getQueryData<DirectMessage[]>(dmKeys.list(userId, generation))
      ?.find((row) => row.id === event.c)?.peer;
    return peer ? [...own, { id: peer.id, name: peer.name }] : own;
  }
  const server = client.getQueryData<ServerDetail>(
    serverKeys.detail(userId, generation, event.s),
  );
  return (server?.members ?? []).map((m) => ({ id: m.user_id, name: m.name }));
}

/** Toast + optional desktop notification for creates in another chat. */
export function useMessageToastsBridge(): void {
  const client = useQueryClient();
  // Keeps the settings cached for a channel without an unread row yet.
  useNotificationSettings();
  // Whether notifications show the message at all (#268).
  useNotificationPrivacy();
  const navigate = useNavigate();
  const me = useSession((s) => s.user?.id);
  const viewingChannelId = useParams({ strict: false }).channelId;
  // An account takes its notifications with it. A window that only just
  // learns who is signed in closes nothing: they belong to all windows.
  useEffect(() => {
    if (me) return () => void closeMessageNotifications({ user: me });
  }, [me]);
  useEffect(() => {
    if (me && viewingChannelId)
      return silenceWhileViewing(me, viewingChannelId);
  }, [me, viewingChannelId]);
  const deliveries = useRef<{
    userId: string | undefined;
    generation: number;
    first: ReturnType<typeof createNotificationDedupe>;
    pacing: ReturnType<typeof createNotificationPacing>;
    /** One sound, one flash for a burst across conversations. */
    sound: ReturnType<typeof createNotificationPacing>;
    attention: ReturnType<typeof createNotificationPacing>;
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
        pacing: createNotificationPacing(),
        sound: createNotificationPacing(),
        attention: createNotificationPacing(),
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
      const focused = typeof document === "undefined" || document.hasFocus();
      const hidden = typeof document !== "undefined" && document.hidden;
      const own = message.author.id === userId;
      const dm = isDmTopic(event.s, event.c);
      const mentioned = !own && mentionsMe(message, userId);
      // Level and mute of the conversation (#242), from the unread rows.
      // A conversation too new for a row yet follows its own setting and
      // its server's, as the row would (#276).
      const row =
        client
          .getQueryData<ReadState[]>(readKey({ userId, generation }))
          ?.find((current) => current.channel_id === event.c) ??
        conversationSetting(
          client.getQueryData<NotificationSetting[]>(
            settingsKey({ userId, generation }),
          ),
          event.c,
          dm ? null : event.s,
          serverNow(),
        );
      if (!wantsNotification(row, { dm, mentioned }, serverNow())) return;
      // The desktop app's taskbar button flashes for a direct message while
      // the window is not in front, whatever the toast and desktop settings
      // say (a mute or the level "nothing" returned above), once for a burst.
      if (
        dm &&
        !own &&
        (hidden || !focused) &&
        serverNow() - Date.parse(message.created_at) < 30_000 &&
        deliveries.current!.attention.take("", Date.now())
      )
        requestDesktopAttention();
      const decision = messageNotificationDecision({
        toastEnabled: settings.messageToasts,
        desktopEnabled: settings.desktopNotify,
        hidden,
        focused,
        type: event.t,
        own,
        channelId: event.c,
        viewingChannelId,
      });
      if (!decision.toast && !decision.desktop) return;
      const label = channelLabel(client, userId, generation, event, dm);
      const preview = previewText(
        readableMentions(
          message.content,
          conversationMembers(client, userId, generation, event, dm),
        ),
        asAttachmentList(message.attachments).length > 0,
        Boolean(message.gif),
      );
      if (decision.toast)
        useMessageToasts.getState().push({
          channelId: event.c,
          serverId: event.s,
          dm,
          channelLabel: label,
          author: message.author.name,
          mentioned,
          preview,
        });
      const now = Date.now();
      const pacing = deliveries.current!.pacing;
      // Replays older than the live delivery window do not generate a burst.
      // Age is measured on the server clock: a fast local clock must not
      // suppress every live notification.
      if (
        decision.desktop &&
        serverNow() - Date.parse(message.created_at) < 30_000 &&
        pacing.take(event.c, now)
      ) {
        const channelId = event.c;
        void showMessageNotification(
          {
            // The in-app toast above stays as it is: it shows on an open
            // page, never on a locked screen.
            ...messageNotificationText(
              client,
              { userId, generation },
              { author: message.author.name, mentioned, label, preview },
            ),
            channelId,
            messageId: message.id,
            createdAt: message.created_at,
            path: conversationPath(dm, event.s, channelId),
            user: userId,
          },
          () => {
            if (!stampHolds({ userId, generation })) return;
            window.focus();
            focusDesktopWindow();
            if (dm) {
              void navigate({ to: "/d/$channelId", params: { channelId } });
            } else {
              void navigate({
                to: "/s/$serverId/c/$channelId",
                params: { serverId: event.s, channelId },
              });
            }
          },
        ).then((shown) => {
          // Read in another window instead of announced. Should that window
          // be gone a second later, the next message is not held back.
          if (shown === "viewed") pacing.giveBack(channelId, now);
          // A phone's notification sounds as the device is set. On a
          // desktop it is silent; this window showed it, so it plays the
          // cue, once for a burst and never into a deafened call.
          if (
            (shown === "worker" || shown === "page") &&
            !useInstallation.getState().mobile &&
            !useVoice.getState().deafened &&
            stampHolds({ userId, generation }) &&
            deliveries.current?.sound.take("", Date.now())
          )
            playMessageSound();
        });
      }
    });
  }, [client, me, navigate, viewingChannelId]);
}
