import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadge } from "../messages/useBadge.ts";
import { notificationMenu } from "./notificationTarget.ts";
import { useServerMuted } from "../messages/notificationSettings.ts";
import { useQueryClient } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useId, useRef, useState, type CSSProperties } from "react";

import { useUserId } from "../auth/scope.ts";
import { lastDmStillListed } from "../dms/open.ts";
import { prefetchDms, useDms } from "../dms/queries.ts";
import { lastDmId, useLastDm } from "../dms/lastDm.ts";
import {
  lastChannelsFor,
  railChannel,
  useLastChannel,
} from "../servers/lastChannel.ts";
import { prefetchServer, useServers } from "../servers/queries.ts";
import type { Server } from "../servers/types.ts";
import { useVoiceRoster } from "../voice/roster.ts";
import { ChatIcon, PlusIcon } from "./Icons.tsx";
import { AddServerDialog } from "./ServerDialogs.tsx";
import { tileInitials, tileTint } from "./initials.ts";
import { useNavigationDrawer } from "./navigationDrawer.ts";
import { LIST_ROW, ROW_ITEM, listStop, moveWithinList } from "./rowKeys.ts";

const TILE_PX = 60;

export function ServerRail({
  activeId,
  dmActive,
}: {
  activeId: string | undefined;
  dmActive: boolean;
}) {
  const { data: servers, isPending } = useServers();
  const [adding, setAdding] = useState(false);
  return (
    <nav aria-label="Server" className="server-rail">
      <HomeTile active={dmActive} />
      {isPending ? (
        <RailPlaceholder />
      ) : (
        <ServerList servers={servers ?? []} activeId={activeId} />
      )}
      <div className="server-rail-footer">
        <button
          type="button"
          onClick={() => setAdding(true)}
          title="Server hinzufügen"
          aria-label="Server hinzufügen"
          className="server-create-button"
        >
          <PlusIcon size={20} />
        </button>
      </div>
      <AddServerDialog open={adding} onClose={() => setAdding(false)} />
    </nav>
  );
}

/** Three quiet tiles until the server list arrives. */
function RailPlaceholder() {
  return (
    <div className="server-rail-scroll" aria-hidden="true">
      {[0, 1, 2].map((index) => (
        <div key={index} className="server-rail-placeholder">
          <span className="server-tile server-tile-placeholder" />
        </div>
      ))}
    </div>
  );
}

function HomeTile({ active }: { active: boolean }) {
  const unreadId = useId();
  const client = useQueryClient();
  const userId = useUserId();
  const byUser = useLastDm((state) => state.byUser);
  const remembered = lastDmId(byUser, userId);
  const { data: dms } = useDms();
  const openId = lastDmStillListed(remembered, dms) ? remembered : null;
  const badge = useBadge({ dms: true });
  const drawer = useNavigationDrawer();
  return (
    <div className="server-home-tile">
      <Link
        to={openId ? "/d/$channelId" : "/d"}
        params={openId ? { channelId: openId } : undefined}
        state={drawer.keepOpen}
        replace={drawer.open}
        title="Direktnachrichten"
        aria-label="Direktnachrichten"
        aria-describedby={unreadId}
        aria-current={active ? "page" : undefined}
        onMouseEnter={() => {
          if (userId) prefetchDms(client, userId);
        }}
        onFocus={() => {
          if (userId) prefetchDms(client, userId);
        }}
        className={[
          "relative server-home-link",
          active ? "is-active" : "",
          badge.count > 0 || badge.dot ? "is-unread" : "",
        ].join(" ")}
        onContextMenu={notificationMenu({
          scope: "channel",
          id: "dms",
          name: "Direktnachrichten",
          allDms: true,
        })}
      >
        <ChatIcon size={27} />
        <UnreadBadge id={unreadId} dms rail showDot={false} />
      </Link>
    </div>
  );
}

type Tip = { name: string; summary: string | null; top: number; left: number };

function ServerList({
  servers,
  activeId,
}: {
  servers: Server[];
  activeId: string | undefined;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [focused, setFocused] = useState<string | null>(null);
  const [tip, setTip] = useState<Tip | null>(null);
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: servers.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => TILE_PX,
    getItemKey: (index) => servers[index]?.id ?? index,
    overscan: 6,
  });
  const items = virtualizer.getVirtualItems();
  // One Tab stop for the whole list; ↑ and ↓ move between the servers.
  const stop = listStop(
    items.flatMap((item) => servers[item.index]?.id ?? []),
    focused,
    activeId,
  );
  const showTip = (
    element: HTMLElement,
    name: string,
    summary: string | null,
  ) => {
    const box = element.getBoundingClientRect();
    setTip({ name, summary, top: box.top + box.height / 2, left: box.right });
  };
  return (
    <div
      ref={scrollRef}
      className="server-rail-scroll"
      onScroll={() => setTip(null)}
    >
      <div
        role="list"
        style={{ height: virtualizer.getTotalSize() }}
        className="relative w-full"
        onKeyDown={(event) => {
          if (event.key === "Escape" && tip) {
            event.preventDefault();
            setTip(null);
            return;
          }
          moveWithinList(event);
        }}
      >
        {items.map((row) => {
          const server = servers[row.index];
          if (!server) return null;
          return (
            <div
              key={row.key}
              data-index={row.index}
              role="listitem"
              aria-setsize={servers.length}
              aria-posinset={row.index + 1}
              className="server-rail-row"
              style={{ top: row.start, height: row.size }}
              onFocus={() => setFocused(server.id)}
              {...LIST_ROW}
            >
              <ServerTile
                server={server}
                active={server.id === activeId}
                tabStop={server.id === stop}
                onTip={showTip}
                onTipEnd={() => setTip(null)}
              />
            </div>
          );
        })}
      </div>
      {/* The name and what is new, beside the tile on hover and keyboard
          focus. Screen readers get both from the tile itself. */}
      {tip ? (
        <div
          className="server-tile-tip"
          aria-hidden="true"
          style={{ top: tip.top, left: tip.left }}
        >
          <strong>{tip.name}</strong>
          {tip.summary ? <span>{tip.summary}</span> : null}
        </div>
      ) : null}
    </div>
  );
}

function ServerTile({
  server,
  active,
  tabStop,
  onTip,
  onTipEnd,
}: {
  server: Server;
  active: boolean;
  tabStop: boolean;
  onTip: (element: HTMLElement, name: string, summary: string | null) => void;
  onTipEnd: () => void;
}) {
  const unreadId = useId();
  const client = useQueryClient();
  const userId = useUserId();
  const muted = useServerMuted(server.id);
  const badge = useBadge({ serverId: server.id });
  const byUser = useLastChannel((s) => s.byUser);
  const someoneInVoice = useVoiceRoster(
    (s) => Object.keys(s.byServer[server.id] ?? {}).length > 0,
  );
  const channelId = railChannel(
    lastChannelsFor(byUser, userId)[server.id],
    someoneInVoice,
  );
  const drawer = useNavigationDrawer();
  const summary =
    badge.count > 0
      ? `${badge.count > 99 ? "Mehr als 99" : badge.count} ${badge.count === 1 ? "Erwähnung" : "Erwähnungen"}`
      : badge.dot
        ? "Ungelesene Nachrichten"
        : muted
          ? "Stummgeschaltet"
          : null;
  return (
    <Link
      to={channelId ? "/s/$serverId/c/$channelId" : "/s/$serverId"}
      params={
        channelId ? { serverId: server.id, channelId } : { serverId: server.id }
      }
      // Picked inside the phone drawer: the drawer stays open on this
      // server's channels.
      state={drawer.keepOpen}
      replace={drawer.open}
      aria-label={server.name}
      aria-describedby={unreadId}
      aria-current={active ? "page" : undefined}
      tabIndex={tabStop ? 0 : -1}
      onPointerEnter={(event) => {
        if (event.pointerType === "mouse")
          onTip(event.currentTarget, server.name, summary);
        if (userId) prefetchServer(client, userId, server.id);
      }}
      onPointerLeave={onTipEnd}
      onFocus={(event) => {
        if (event.currentTarget.matches(":focus-visible"))
          onTip(event.currentTarget, server.name, summary);
        if (userId) prefetchServer(client, userId, server.id);
      }}
      onBlur={onTipEnd}
      className={[
        "server-tile",
        active ? "is-active" : "",
        muted ? "is-muted" : "",
        badge.count > 0 || badge.dot ? "is-unread" : "",
      ].join(" ")}
      style={{ "--server-tile": tileTint(server.id) } as CSSProperties}
      onContextMenu={notificationMenu({
        scope: "server",
        id: server.id,
        name: server.name,
      })}
      {...ROW_ITEM}
    >
      {tileInitials(server.name)}
      <UnreadBadge id={unreadId} serverId={server.id} rail showDot={false} />
    </Link>
  );
}
