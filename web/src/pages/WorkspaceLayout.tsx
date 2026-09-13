// Three columns: server rail, channel sidebar for the selected server, and
// the page. Selection is the URL (`/s/$serverId/c/$channelId`), so both
// sidebars re-highlight synchronously with the click; data fills in from the
// query cache (usually already warm from the hover prefetch).

import { useQueryClient } from "@tanstack/react-query";
import { Navigate, Outlet, useParams } from "@tanstack/react-router";

import { ApiError } from "../api/client.ts";
import { ChannelSidebar } from "../components/ChannelSidebar.tsx";
import { RequireUser } from "../components/RequireUser.tsx";
import { ServerRail } from "../components/ServerRail.tsx";
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

  return (
    <div className="flex h-[calc(100dvh-3.5rem)] overflow-hidden bg-neutral-50">
      <ServerRail activeId={serverId} />
      {serverId ? (
        <SelectedServer serverId={serverId} channelId={channelId} />
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
  const { data: server, error } = useServer(serverId);

  if (
    error instanceof ApiError &&
    (error.code === "not_found" || error.code === "forbidden")
  ) {
    // Deleted, or we were removed: drop it from the rail and go home.
    forgetServer(client, serverId);
    return <Navigate to="/" replace />;
  }

  return (
    <>
      {server ? (
        <ChannelSidebar server={server} activeChannelId={channelId} />
      ) : (
        <SidebarSkeleton />
      )}
      <main className="flex min-w-0 flex-1 flex-col overflow-y-auto">
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
      className="flex h-full w-64 shrink-0 flex-col border-r border-neutral-200 bg-white"
    >
      <div className="h-12 border-b border-neutral-200" />
      <div className="flex flex-col gap-2 p-3">
        {[0, 1, 2, 3].map((i) => (
          <div key={i} className="h-5 animate-pulse rounded bg-neutral-100" />
        ))}
      </div>
    </aside>
  );
}
