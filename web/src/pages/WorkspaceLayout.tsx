// Three columns: server rail, channel (or DM) sidebar, and the page.
// Selection is the URL (`/s/$serverId/c/$channelId` or `/d/$channelId`).

import { useQueryClient } from "@tanstack/react-query";
import {
  Outlet,
  useNavigate,
  useParams,
  useRouterState,
} from "@tanstack/react-router";
import { useEffect } from "react";

import { ApiError } from "../api/client.ts";
import { ChannelSidebar } from "../components/ChannelSidebar.tsx";
import { DmSidebar } from "../components/DmSidebar.tsx";
import { RequireUser } from "../components/RequireUser.tsx";
import { ServerRail } from "../components/ServerRail.tsx";
import { useDms } from "../dms/queries.ts";
import { forgetServer, useServer } from "../servers/queries.ts";

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

  return (
    <div className="flex h-[calc(100dvh-3.5rem)] overflow-hidden bg-neutral-50 dark:bg-neutral-950">
      <ServerRail activeId={serverId} dmActive={onDms} />
      {serverId ? (
        <SelectedServer serverId={serverId} channelId={channelId} />
      ) : onDms ? (
        <SelectedDms channelId={channelId} />
      ) : (
        <main className="flex-1 overflow-y-auto">
          <Outlet />
        </main>
      )}
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
  const { data: server, error } = useServer(serverId);
  const gone =
    error instanceof ApiError &&
    (error.code === "not_found" || error.code === "forbidden");

  useEffect(() => {
    if (!gone) return;
    // Deleted, or we were removed: drop it from the rail and go home. The
    // cached detail is removed only after this component is gone, otherwise
    // the still-mounted query would refetch and 404 again.
    forgetServer(client, serverId, { keepDetail: true });
    void navigate({ to: "/", replace: true }).then(() =>
      forgetServer(client, serverId),
    );
  }, [gone, client, navigate, serverId]);

  if (gone) return null;

  return (
    <>
      {server ? (
        <ChannelSidebar server={server} activeChannelId={channelId} />
      ) : (
        <SidebarSkeleton />
      )}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <Outlet />
      </main>
    </>
  );
}

function SelectedDms({ channelId }: { channelId: string | undefined }) {
  const { data: dms } = useDms();

  return (
    <>
      {dms ? (
        <DmSidebar dms={dms} activeChannelId={channelId} />
      ) : (
        <SidebarSkeleton />
      )}
      <main className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <Outlet />
      </main>
    </>
  );
}

/** Same footprint as the real sidebar so nothing jumps when data arrives. */
function SidebarSkeleton() {
  return (
    <aside
      aria-hidden
      className="flex h-full w-64 shrink-0 flex-col border-r border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
    >
      <div className="h-12 border-b border-neutral-200 dark:border-neutral-700" />
      <div className="flex flex-col gap-2 p-3">
        {[0, 1, 2, 3].map((i) => (
          <div
            key={i}
            className="h-5 animate-pulse rounded bg-neutral-100 dark:bg-neutral-800"
          />
        ))}
      </div>
    </aside>
  );
}
