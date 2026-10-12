import {
  Link,
  Outlet,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";

import { useReadBridge } from "../messages/readState.ts";
import { logout, useSession } from "../auth/session.ts";
import { leaveVoice, stopWatching, useVoice } from "../voice/session.ts";
import { useCallWakeLock } from "../voice/wakeLock.ts";
import {
  useAuthenticatedSubscriptions,
  useGatewaySession,
} from "../ws/useGateway.ts";
import { useIdlePresence } from "../ws/useLive.ts";
import { useMessageToastsBridge } from "../messages/useMessageToasts.ts";
import { Avatar } from "./Avatar.tsx";
import { GearIcon } from "./Icons.tsx";
import { isChromeLongPress } from "./longPress.ts";
import { MessageToasts } from "./MessageToasts.tsx";
import { Toasts } from "./Toasts.tsx";
import { VoiceSettingsDialog } from "./VoiceSettingsDialog.tsx";
import { VoiceSessionControls } from "./VoiceSessionControls.tsx";
import { SessionRecoveryNotice } from "./SessionRecoveryNotice.tsx";

function AuthenticatedRealtime() {
  useAuthenticatedSubscriptions();
  useReadBridge();
  return null;
}

export function AppShell() {
  const user = useSession((state) => state.user);
  const navigate = useNavigate();
  const workspace = useRouterState({
    select: (state) =>
      state.matches.some((match) => match.routeId === "/workspace"),
  });
  const activeMedia = useVoice(
    (state) =>
      state.status === "joined" || state.watching || state.playbackBlocked,
  );
  useGatewaySession(user?.id ?? null);
  useIdlePresence(user?.id ?? null);
  useMessageToastsBridge();
  useCallWakeLock(
    useVoice((state) => state.status === "joined" || state.watching),
  );

  const onLogout = () => {
    // Store flips first, so the header and guards react before the request
    // even leaves; the navigation is client-side.
    leaveVoice();
    stopWatching();
    void logout();
    void navigate({ to: "/login" });
  };

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
      {!workspace ? (
        <header className="app-header">
          <div className="flex h-full items-center justify-between px-4">
            <Link to="/" className="text-lg font-semibold tracking-tight">
              Gelabber
            </Link>
            {user ? (
              <nav className="flex items-center gap-3 text-sm">
                <Link
                  to="/settings"
                  className="inline-flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100"
                  activeProps={{
                    className:
                      "bg-neutral-100 dark:bg-neutral-800 text-neutral-900 dark:text-neutral-100",
                  }}
                >
                  <GearIcon size={16} />
                  Einstellungen
                </Link>
                <Link
                  to="/profile"
                  className="flex items-center gap-2 rounded-full py-1 pr-3 pl-1 hover:bg-neutral-100 dark:hover:bg-neutral-800"
                  activeProps={{
                    className: "bg-neutral-100 dark:bg-neutral-800",
                  }}
                >
                  <Avatar name={user.name} url={user.avatar_url} />
                  <span className="max-w-40 truncate font-medium">
                    {user.name}
                  </span>
                </Link>
                <button
                  type="button"
                  onClick={onLogout}
                  className="rounded-lg px-3 py-1.5 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100"
                >
                  Abmelden
                </button>
              </nav>
            ) : null}
          </div>
        </header>
      ) : null}
      <div id="app-content" tabIndex={workspace ? undefined : -1}>
        <SessionRecoveryNotice />
        <Outlet />
      </div>
      {user ? <AuthenticatedRealtime key={user.id} /> : null}
      <Toasts />
      <MessageToasts />
      {user ? <VoiceSettingsDialog /> : null}
      {user ? <VoiceSessionControls /> : null}
    </div>
  );
}
