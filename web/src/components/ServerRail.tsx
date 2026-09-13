// Left-most column: one round tile per server, virtualised so a hundred
// servers scroll as smoothly as five. The active tile follows the URL, so a
// click highlights before any request finishes.

import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";

import { useLastChannel } from "../servers/lastChannel.ts";
import { prefetchServer, useServers } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { PlusIcon } from "./Icons.tsx";
import { CreateServerDialog } from "./ServerDialogs.tsx";
import { initials } from "./initials.ts";

const TILE_PX = 56;

export function ServerRail({ activeId }: { activeId: string | undefined }) {
  const { data: servers } = useServers();
  const [creating, setCreating] = useState(false);

  return (
    <nav
      aria-label="Server"
      className="flex h-full w-18 shrink-0 flex-col items-center border-r border-neutral-200 bg-neutral-100"
    >
      <ServerList servers={servers ?? []} activeId={activeId} />
      <div className="flex w-full justify-center border-t border-neutral-200 py-2">
        <button
          type="button"
          onClick={() => setCreating(true)}
          title="Server erstellen"
          aria-label="Server erstellen"
          className="flex size-12 items-center justify-center rounded-2xl bg-white text-emerald-700 shadow-sm transition hover:rounded-xl hover:bg-emerald-600 hover:text-white"
        >
          <PlusIcon size={20} />
        </button>
      </div>
      <CreateServerDialog open={creating} onClose={() => setCreating(false)} />
    </nav>
  );
}

function ServerList({
  servers,
  activeId,
}: {
  servers: Server[];
  activeId: string | undefined;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: servers.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TILE_PX,
    overscan: 6,
  });

  return (
    <div ref={scrollRef} className="w-full flex-1 overflow-y-auto py-2">
      <div
        style={{ height: virtualizer.getTotalSize() }}
        className="relative w-full"
      >
        {virtualizer.getVirtualItems().map((row) => {
          const server = servers[row.index];
          if (!server) return null;
          return (
            <div
              key={server.id}
              data-index={row.index}
              className="absolute inset-x-0 flex justify-center"
              style={{ top: row.start, height: row.size }}
            >
              <ServerTile server={server} active={server.id === activeId} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function ServerTile({ server, active }: { server: Server; active: boolean }) {
  const client = useQueryClient();
  const lastChannelId = useLastChannel((s) => s.byServer[server.id]);

  return (
    <div className="relative flex h-14 items-center">
      <span
        aria-hidden
        className={[
          "absolute -left-3 w-1 rounded-r-full bg-neutral-900 transition-all",
          active ? "h-8" : "h-0",
        ].join(" ")}
      />
      <Link
        to={lastChannelId ? "/s/$serverId/c/$channelId" : "/s/$serverId"}
        params={
          lastChannelId
            ? { serverId: server.id, channelId: lastChannelId }
            : { serverId: server.id }
        }
        title={server.name}
        aria-label={server.name}
        aria-current={active ? "page" : undefined}
        onMouseEnter={() => prefetchServer(client, server.id)}
        onFocus={() => prefetchServer(client, server.id)}
        className={[
          "flex size-12 items-center justify-center text-sm font-semibold transition-all select-none",
          active
            ? "rounded-xl bg-neutral-900 text-white"
            : "rounded-2xl bg-white text-neutral-700 shadow-sm hover:rounded-xl hover:bg-neutral-900 hover:text-white",
        ].join(" ")}
      >
        {initials(server.name)}
      </Link>
    </div>
  );
}
