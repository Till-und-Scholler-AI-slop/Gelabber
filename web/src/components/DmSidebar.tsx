import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef } from "react";

import type { DirectMessage } from "../dms/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { UserPanel } from "./UserPanel.tsx";

const ROW_PX = 48;

export function DmSidebar({
  dms,
  activeChannelId,
}: {
  dms: DirectMessage[];
  activeChannelId: string | undefined;
}) {
  return (
    <aside
      aria-label="Direktnachrichten"
      className="channel-sidebar dm-sidebar"
    >
      <Link to="/" className="sidebar-brand">
        Gelabber
      </Link>
      <header className="dm-sidebar-heading">
        <h2>Nachrichten</h2>
        <p>Deine Unterhaltungen.</p>
      </header>
      {dms.length === 0 ? (
        <p className="sidebar-empty">
          Noch keine Unterhaltungen. Öffne eine über die Mitgliederliste.
        </p>
      ) : (
        <DmList dms={dms} activeChannelId={activeChannelId} />
      )}
      <UserPanel contextId={activeChannelId} />
    </aside>
  );
}

function DmList({
  dms,
  activeChannelId,
}: {
  dms: DirectMessage[];
  activeChannelId: string | undefined;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: dms.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    getItemKey: (index) => dms[index]?.id ?? index,
    overscan: 10,
  });
  return (
    <div ref={scrollRef} className="sidebar-list-scroll">
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((item) => {
          const dm = dms[item.index];
          if (!dm) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              className="sidebar-virtual-row"
              style={{ top: item.start, height: item.size }}
            >
              <DmRow dm={dm} active={dm.id === activeChannelId} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

function DmRow({ dm, active }: { dm: DirectMessage; active: boolean }) {
  const status = usePresenceStore((state) =>
    presenceOf(state.byServer, dm.id, dm.peer.id),
  );
  return (
    <Link
      to="/d/$channelId"
      params={{ channelId: dm.id }}
      aria-current={active ? "page" : undefined}
      className={`dm-row ${active ? "is-active" : ""}`}
    >
      <PresenceAvatar
        name={dm.peer.name}
        url={dm.peer.avatar_url}
        status={status}
      />
      <span>{dm.peer.name}</span>
    </Link>
  );
}
