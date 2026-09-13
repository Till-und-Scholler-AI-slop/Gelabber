// Three columns: server rail, channel sidebar for the selected server, and
// the page. Selection is the URL (`/s/$serverId/c/$channelId`), so both
// sidebars re-highlight synchronously with the click; data fills in from the
// query cache (usually already warm from the hover prefetch).

import { useQueryClient } from "@tanstack/react-query";
import { Outlet, useNavigate, useParams } from "@tanstack/react-router";
import { useEffect } from "react";

import { ApiError } from "../api/client.ts";
import { ChannelSidebar } from "../components/ChannelSidebar.tsx";
import { RequireUser } from "../components/RequireUser.tsx";
import { ServerRail } from "../components/ServerRail.tsx";
import { forgetServer, useServer } from "../servers/queries.ts";
import { useGatewayTopics } from "../ws/useGateway.ts";

export function WorkspaceLayout() {
  return (
    <RequireUser>
      <Workspace />
    </RequireUser>
  );
}

function Workspace() {
  const { serverId, channelId } = useParams({ strict: false });
  useGatewayTopics(serverId, channelId);

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
