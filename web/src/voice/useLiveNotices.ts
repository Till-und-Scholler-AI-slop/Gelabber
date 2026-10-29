// Feeds the gateway's voice frames to the Live notices (liveNotices.ts) and
// delivers them as a message notification is delivered (useMessageToasts.ts):
// the in-app toast, the system notification, the cue.

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { serverNow } from "../api/client.ts";
import { scopeGeneration, stampHolds } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { focusDesktopWindow } from "../messages/desktopShell.ts";
import {
  hidesContent,
  privacyKey,
  useNotificationPrivacy,
  type NotificationPrivacy,
} from "../messages/notificationPrivacy.ts";
import {
  settingOf,
  settingsKey,
  useNotificationSettings,
  type NotificationSetting,
} from "../messages/notificationSettings.ts";
import { useMessageToasts } from "../messages/toasts.ts";
import { useInstallation } from "../pwa/install.ts";
import { showMessageNotification } from "../pwa/notifications.ts";
import { ensureServer } from "../servers/queries.ts";
import { getGateway } from "../ws/client.ts";
import { playLiveSound } from "./callSounds.ts";
import {
  createLiveNoticeMemory,
  createLiveNotices,
  type LiveNoticeMemory,
} from "./liveNotices.ts";
import { useVoiceRoster } from "./roster.ts";
import { useMediaSettings } from "./settings.ts";
import { useVoice } from "./voiceStore.ts";

export function useLiveNotices(): void {
  const client = useQueryClient();
  // The server's mute and level, and whether notifications show content.
  useNotificationSettings();
  useNotificationPrivacy();
  const navigate = useNavigate();
  const me = useSession((s) => s.user?.id);
  const viewingChannelId = useParams({ strict: false }).channelId;
  const memory = useRef<{
    userId: string;
    generation: number;
    notices: LiveNoticeMemory;
  } | null>(null);

  useEffect(() => {
    const userId = me;
    if (!userId) return;
    const generation = scopeGeneration();
    if (
      memory.current?.userId !== userId ||
      memory.current.generation !== generation
    )
      memory.current = {
        userId,
        generation,
        notices: createLiveNoticeMemory(),
      };
    const stamp = { userId, generation };
    const holds = () => stampHolds(stamp);
    const handle = createLiveNotices(
      {
        me: userId,
        now: Date.now,
        serverNow,
        server: (serverId) => ensureServer(client, stamp, serverId),
        setting: (serverId) =>
          settingOf(
            client.getQueryData<NotificationSetting[]>(settingsKey(stamp)),
            "server",
            serverId,
          ),
        here: (serverId, channelId) => {
          const call = useVoice.getState();
          if (
            call.status === "joined" &&
            call.serverId === serverId &&
            call.channelId === channelId
          )
            return true;
          const seat = useVoiceRoster.getState().byServer[serverId]?.[userId];
          return seat?.channelId === channelId;
        },
        device: () => {
          const settings = useMediaSettings.getState();
          return {
            toasts: settings.messageToasts,
            desktop: settings.desktopNotify,
            hidden: typeof document !== "undefined" && document.hidden,
            focused: typeof document === "undefined" || document.hasFocus(),
            viewingChannelId,
            mobile: useInstallation.getState().mobile,
            deafened: useVoice.getState().deafened,
            hideContent: hidesContent(
              client.getQueryState<NotificationPrivacy>(privacyKey(stamp)),
            ),
          };
        },
        holds,
        toast: (toast) => useMessageToasts.getState().push(toast),
        notify: showMessageNotification,
        sound: playLiveSound,
        open: (serverId, channelId) => {
          window.focus();
          focusDesktopWindow();
          void navigate({
            to: "/s/$serverId/c/$channelId",
            params: { serverId, channelId },
          });
        },
      },
      memory.current.notices,
    );
    return getGateway().onSig((event) => {
      if (holds()) handle(event);
    });
  }, [client, me, navigate, viewingChannelId]);
}
