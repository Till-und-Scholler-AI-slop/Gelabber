// Second column on Home: the signed-in user's 1:1 DMs. Highlight follows
// the URL; rows are virtualised like server channels.

import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef } from "react";

import type { DirectMessage } from "../dms/types.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { PresenceAvatar } from "./PresenceAvatar.tsx";

const ROW_PX = 44;

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
      className="flex h-full w-64 shrink-0 flex-col border-r border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
    >
      <header className="flex h-12 items-center border-b border-neutral-200 dark:border-neutral-700 px-3">
        <h2 className="truncate font-semibold tracking-tight">
          Direktnachrichten
        </h2>
      </header>
      {dms.length === 0 ? (
        <p className="flex-1 px-3 py-4 text-sm text-neutral-500 dark:text-neutral-400">
          Noch keine Unterhaltungen. Öffne eine über die Mitgliederliste.
        </p>
      ) : (
        <DmList dms={dms} activeChannelId={activeChannelId} />
      )}
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
  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: dms.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    getItemKey: (index) => dms[index]?.id ?? index,
    overscan: 10,
  });

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto px-2 py-2">
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((item) => {
          const dm = dms[item.index];
          if (!dm) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              className="absolute inset-x-0"
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
  const status = usePresenceStore((s) =>
    presenceOf(s.byServer, dm.id, dm.peer.id),
  );
  return (
    <Link
      to="/d/$channelId"
      params={{ channelId: dm.id }}
      aria-current={active ? "page" : undefined}
      className={[
        "flex h-[42px] items-center gap-2 rounded-md px-2 text-sm",
        active
          ? "bg-neutral-900 dark:bg-neutral-700 text-white"
          : "text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100",
      ].join(" ")}
    >
      <PresenceAvatar
        name={dm.peer.name}
        url={dm.peer.avatar_url}
        status={status}
      />
      <span className="truncate">{dm.peer.name}</span>
    </Link>
  );
}
