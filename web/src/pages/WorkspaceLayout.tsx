// URL-selected workspace. Queries and cache eviction retain their existing scope.
import { useQueryClient } from "@tanstack/react-query";
import {
  Outlet,
  useNavigate,
  useParams,
  useRouterState,
} from "@tanstack/react-router";
import { useEffect, useId, useState, type ReactNode } from "react";

import { ApiError } from "../api/client.ts";
import { useUserId } from "../auth/scope.ts";
import { ChannelSidebar } from "../components/ChannelSidebar.tsx";
import { DmSidebar } from "../components/DmSidebar.tsx";
import { ChevronIcon } from "../components/Icons.tsx";
import { RequireUser } from "../components/RequireUser.tsx";
import { ServerRail } from "../components/ServerRail.tsx";
import { SidebarPlaceholder } from "../components/SidebarPlaceholder.tsx";
import { UserPanel } from "../components/UserPanel.tsx";
import { WorkspaceDrawer } from "../components/WorkspaceNavigation.tsx";
import { notify } from "../components/toasts.ts";
import { useDms } from "../dms/queries.ts";
import { useReadState, useReadingChannel } from "../messages/readState.ts";
import { forgetServer, useServer, useServers } from "../servers/queries.ts";

export function WorkspaceLayout() {
  return (
    <RequireUser>
      <Workspace />
    </RequireUser>
  );
}

function Workspace() {
  const { serverId, channelId } = useParams({ strict: false });
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const onDms = pathname === "/d" || pathname.startsWith("/d/");
  // Before the early returns so hooks keep their order; the rail shares it.
  const { data: servers } = useServers();

  if (serverId)
    return <SelectedServer serverId={serverId} channelId={channelId} />;
  if (onDms) return <SelectedDms channelId={channelId} />;
  return (
    <WorkspaceFrame
      sidebar={
        <aside className="channel-sidebar" aria-label="Navigation">
          <div className="sidebar-brand">Gelabber</div>
          <div className="sidebar-empty">
            Dein Platz für gemeinsame Abende.
            {/* Only once the list says so: never flash it at members. */}
            {servers?.length === 0
              ? " Erstelle einen Server mit dem Plus in der Serverleiste oder öffne einen Einladungslink."
              : null}
          </div>
          <UserPanel />
        </aside>
      }
    />
  );
}

function WorkspaceFrame({
  sidebar,
  serverId,
  dmActive = false,
}: {
  sidebar: ReactNode;
  serverId?: string;
  dmActive?: boolean;
}) {
  const [navigationOpen, setNavigationOpen] = useState(false);
  const navigationId = useId();
  const unreadId = useId();
  // Phones keep every unread badge inside the closed drawer.
  const reading = useReadingChannel();
  const { data: readRows } = useReadState();
  const unread = (readRows ?? []).some(
    (row) => row.channel_id !== reading && row.unread_count > 0,
  );
  return (
    <div className="workspace">
      <WorkspaceDrawer
        id={navigationId}
        open={navigationOpen}
        onClose={() => setNavigationOpen(false)}
        title="Navigation"
        breakpoint={800}
        className="workspace-navigation"
        closeOnNavigate
      >
        <ServerRail activeId={serverId} dmActive={dmActive} />
        {sidebar}
      </WorkspaceDrawer>
      <div className="workspace-main">
        <header className="workspace-mobile-topbar">
          <button
            type="button"
            className="workspace-navigation-trigger"
            aria-label="Navigation öffnen"
            aria-expanded={navigationOpen}
            aria-controls={navigationId}
            aria-describedby={unread ? unreadId : undefined}
            onClick={() => setNavigationOpen(true)}
          >
            <ChevronIcon size={21} />
            {unread ? (
              <span
                id={unreadId}
                role="img"
                aria-label="Ungelesene Nachrichten"
                className="workspace-navigation-unread"
              />
            ) : null}
            <span>Gelabber</span>
          </button>
        </header>
        <main
          id="workspace-content"
          tabIndex={-1}
          className="workspace-content"
        >
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function SelectedServer({
  serverId,
  channelId,
}: {
  serverId: string;
  channelId: string | undefined;
}) {
  const client = useQueryClient();
  const navigate = useNavigate();
  const userId = useUserId();
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  const gone =
    error instanceof ApiError &&
    (error.code === "not_found" || error.code === "forbidden");

  useEffect(() => {
    if (!gone || !userId) return;
    notify("Server nicht gefunden oder kein Zugriff.");
    forgetServer(client, userId, serverId, { keepDetail: true });
    void navigate({ to: "/", replace: true }).then(() =>
      forgetServer(client, userId, serverId),
    );
  }, [gone, client, navigate, serverId, userId]);

  if (gone) return null;
  return (
    <WorkspaceFrame
      serverId={serverId}
      sidebar={
        server ? (
          <ChannelSidebar server={server} activeChannelId={channelId} />
        ) : (
          <SidebarPlaceholder
            failure={
              error
                ? {
                    message: "Die Kanäle konnten gerade nicht geladen werden.",
                    pending: isFetching,
                    onRetry: () => void refetch(),
                  }
                : undefined
            }
          />
        )
      }
    />
  );
}

function SelectedDms({ channelId }: { channelId: string | undefined }) {
  const { data: dms, error, isFetching, refetch } = useDms();
  return (
    <WorkspaceFrame
      dmActive
      sidebar={
        dms ? (
          <DmSidebar dms={dms} activeChannelId={channelId} />
        ) : (
          <SidebarPlaceholder
            failure={
              error
                ? {
                    message:
                      "Deine Unterhaltungen konnten gerade nicht geladen werden.",
                    pending: isFetching,
                    onRetry: () => void refetch(),
                  }
                : undefined
            }
          />
        )
      }
    />
  );
}
