import { Link, Outlet, useRouterState } from "@tanstack/react-router";

import { useReadBridge } from "../messages/readState.ts";
import { useUnreadIndicators } from "../messages/unreadIndicators.ts";
import { useNotificationSettingsBridge } from "../messages/notificationSettings.ts";
import { useSession } from "../auth/session.ts";
import { useVoice } from "../voice/session.ts";
import { useCallWakeLock } from "../voice/wakeLock.ts";
import {
  useAuthenticatedSubscriptions,
  useGatewaySession,
} from "../ws/useGateway.ts";
import { useFocusReport, useIdlePresence } from "../ws/useLive.ts";
import { usePushSubscription } from "../pwa/push.ts";
import { useMessageToastsBridge } from "../messages/useMessageToasts.ts";
import { ConfirmHost } from "./ConfirmHost.tsx";
import { ConnectionNotice } from "./ConnectionNotice.tsx";
import { InvitePromptHost } from "./InviteDialog.tsx";
import { isChromeLongPress } from "./longPress.ts";
import { MessageToasts } from "./MessageToasts.tsx";
import { NotificationSettingsHost } from "./NotificationSettingsDialog.tsx";
import { Toasts } from "./Toasts.tsx";
import { VoiceSettingsDialog } from "./VoiceSettingsDialog.tsx";
import { VoiceSessionControls } from "./VoiceSessionControls.tsx";
import { SessionRecoveryNotice } from "./SessionRecoveryNotice.tsx";

function AuthenticatedRealtime() {
  useAuthenticatedSubscriptions();
  useReadBridge();
  useUnreadIndicators();
  useNotificationSettingsBridge();
  return null;
}

export function AppShell() {
  const user = useSession((state) => state.user);
  const workspace = useRouterState({
    select: (state) =>
      state.matches.some((match) => match.routeId === "/workspace"),
  });
  const bare = useRouterState({
    select: (state) => state.matches.some((match) => match.staticData.bare),
  });
  const activeMedia = useVoice(
    (state) =>
      state.status === "joined" ||
      state.watching ||
      state.playbackBlocked ||
      state.lostCall !== null,
  );
  useGatewaySession(user?.id ?? null);
  useIdlePresence(user?.id ?? null);
  useFocusReport(user?.id ?? null);
  usePushSubscription(user?.id ?? null);
  useMessageToastsBridge();
  useCallWakeLock(
    useVoice((state) => state.status === "joined" || state.watching),
  );

  return (
    <div
      className={`app-shell ${workspace ? "app-shell-workspace" : ""} ${activeMedia ? "has-active-media" : ""}`}
      onContextMenu={(event) => {
        // Android's link and image menu; index.css covers selection and iOS.
        if (
          isChromeLongPress(
            event.nativeEvent,
            window.matchMedia("(pointer: coarse)").matches,
          )
        )
          event.preventDefault();
      }}
    >
      <a
        className="shell-skip-link"
        href={workspace ? "#workspace-content" : "#app-content"}
      >
        Zum Inhalt springen
      </a>
      {!workspace && !bare ? (
        <header className="app-header">
          <div className="flex h-full items-center justify-between px-4">
            <Link to="/" className="text-lg font-semibold tracking-tight">
              Gelabber
            </Link>
          </div>
        </header>
      ) : null}
      <div id="app-content" tabIndex={workspace ? undefined : -1}>
        <SessionRecoveryNotice>
          <Outlet />
        </SessionRecoveryNotice>
      </div>
      {user ? <AuthenticatedRealtime key={user.id} /> : null}
      {user ? <ConnectionNotice /> : null}
      <Toasts />
      <MessageToasts />
      <ConfirmHost />
      {user ? <VoiceSettingsDialog /> : null}
      {user ? (
        <NotificationSettingsHost key={`notifications-${user.id}`} />
      ) : null}
      {user ? <InvitePromptHost /> : null}
      {user ? <VoiceSessionControls /> : null}
    </div>
  );
}
