import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";

import { useUserId } from "../auth/scope.ts";
import { lastDmStillListed } from "../dms/open.ts";
import { prefetchDms, useDms } from "../dms/queries.ts";
import { lastDmId, useLastDm } from "../dms/lastDm.ts";
import { prefetchServer, useServers } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { ChatIcon, PlusIcon } from "./Icons.tsx";
import { CreateServerDialog } from "./ServerDialogs.tsx";
import { initials } from "./initials.ts";

const TILE_PX = 60;

export function ServerRail({
  activeId,
  dmActive,
}: {
  activeId: string | undefined;
  dmActive: boolean;
}) {
  const { data: servers } = useServers();
  const [creating, setCreating] = useState(false);
  return (
    <nav aria-label="Server" className="server-rail">
      <HomeTile active={dmActive} />
      <ServerList servers={servers ?? []} activeId={activeId} />
      <div className="server-rail-footer">
        <button
          type="button"
          onClick={() => setCreating(true)}
          title="Server erstellen"
          aria-label="Server erstellen"
          className="server-create-button"
        >
          <PlusIcon size={20} />
        </button>
      </div>
      <CreateServerDialog open={creating} onClose={() => setCreating(false)} />
    </nav>
  );
}

function HomeTile({ active }: { active: boolean }) {
  const client = useQueryClient();
  const userId = useUserId();
  const byUser = useLastDm((state) => state.byUser);
  const remembered = lastDmId(byUser, userId);
  const { data: dms } = useDms();
  const openId = lastDmStillListed(remembered, dms) ? remembered : null;
  return (
    <div className="server-home-tile">
      <Link
        to={openId ? "/d/$channelId" : "/d"}
        params={openId ? { channelId: openId } : undefined}
        title="Direktnachrichten"
        aria-label="Direktnachrichten"
        aria-current={active ? "page" : undefined}
        onMouseEnter={() => {
          if (userId) prefetchDms(client, userId);
        }}
        onFocus={() => {
          if (userId) prefetchDms(client, userId);
        }}
        className={`server-home-link ${active ? "is-active" : ""}`}
      >
        <ChatIcon size={27} />
      </Link>
    </div>
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
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: servers.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TILE_PX,
    getItemKey: (index) => servers[index]?.id ?? index,
    overscan: 6,
  });
  return (
    <div ref={scrollRef} className="server-rail-scroll">
      <div
        style={{ height: virtualizer.getTotalSize() }}
        className="relative w-full"
      >
        {virtualizer.getVirtualItems().map((row) => {
          const server = servers[row.index];
          if (!server) return null;
          return (
            <div
              key={row.key}
              data-index={row.index}
              className="server-rail-row"
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
  const userId = useUserId();
  return (
    <Link
      to="/s/$serverId"
      params={{ serverId: server.id }}
      title={server.name}
      aria-label={server.name}
      aria-current={active ? "page" : undefined}
      onMouseEnter={() => {
        if (userId) prefetchServer(client, userId, server.id);
      }}
      onFocus={() => {
        if (userId) prefetchServer(client, userId, server.id);
      }}
      className={`server-tile ${active ? "is-active" : ""}`}
    >
      {initials(server.name)}
    </Link>
  );
}
