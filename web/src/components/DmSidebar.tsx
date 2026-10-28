import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadge } from "../messages/useBadge.ts";
import { useNotificationMenu } from "./notificationTarget.ts";
import { useChannelMuted } from "../messages/notificationSettings.ts";
import { Link } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef, useState } from "react";
import { createPortal } from "react-dom";

import { useUserId } from "../auth/scope.ts";
import type { DirectMessage } from "../dms/types.ts";
import { useHasDraft } from "../messages/drafts.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { PencilIcon, PlusIcon } from "./Icons.tsx";
import { NewMessageDialog } from "./NewMessageDialog.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { UserPanel } from "./UserPanel.tsx";
import { useNavigationDrawer } from "./navigationDrawer.ts";
import { LIST_ROW, ROW_ITEM, listStop, moveWithinList } from "./rowKeys.ts";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";

const ROW_PX = 48;

export function DmSidebar({
  dms,
  activeChannelId,
}: {
  dms: DirectMessage[];
  activeChannelId: string | undefined;
}) {
  const [writing, setWriting] = useState(false);
  const drawer = useNavigationDrawer();
  // From the phone drawer the dialog takes the drawer's place.
  const startNew = () => {
    if (drawer.open) void drawer.hide().then(() => setWriting(true));
    else setWriting(true);
  };
  return (
    <aside
      aria-label="Direktnachrichten"
      className="channel-sidebar dm-sidebar"
    >
      <header className="dm-sidebar-heading">
        <h2>Nachrichten</h2>
        <IconButton
          label="Neue Nachricht"
          icon={<PlusIcon size={18} />}
          size="sm"
          onClick={startNew}
        />
      </header>
      {dms.length === 0 ? (
        <div className="sidebar-empty dm-sidebar-empty">
          <p>Noch keine Unterhaltungen.</p>
          <Button size="sm" icon={<PlusIcon size={16} />} onClick={startNew}>
            Neue Nachricht
          </Button>
        </div>
      ) : (
        <DmList dms={dms} activeChannelId={activeChannelId} />
      )}
      <UserPanel contextId={activeChannelId} />
      {typeof document === "undefined"
        ? null
        : createPortal(
            <NewMessageDialog
              open={writing}
              onClose={() => setWriting(false)}
            />,
            document.body,
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
  const [focused, setFocused] = useState<string | null>(null);
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: dms.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_PX,
    getItemKey: (index) => dms[index]?.id ?? index,
    overscan: 10,
  });
  const items = virtualizer.getVirtualItems();
  // One Tab stop for the list; ↑ and ↓ move between the conversations.
  const stop = listStop(
    items.flatMap((item) => dms[item.index]?.id ?? []),
    focused,
    activeChannelId,
  );
  return (
    <div ref={scrollRef} className="sidebar-list-scroll">
      <div
        role="list"
        aria-label="Unterhaltungen"
        style={{ height: virtualizer.getTotalSize() }}
        className="relative"
        onKeyDown={moveWithinList}
      >
        {items.map((item) => {
          const dm = dms[item.index];
          if (!dm) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              role="listitem"
              aria-setsize={dms.length}
              aria-posinset={item.index + 1}
              className="sidebar-virtual-row"
              style={{ top: item.start, height: item.size }}
              onFocus={() => setFocused(dm.id)}
              {...LIST_ROW}
            >
              <DmRow
                dm={dm}
                active={dm.id === activeChannelId}
                tabStop={dm.id === stop}
              />
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function DmRow({
  dm,
  active,
  tabStop = true,
}: {
  dm: DirectMessage;
  active: boolean;
  /** In a list with one Tab stop: whether it is this row's. */
  tabStop?: boolean;
}) {
  const status = usePresenceStore((state) =>
    presenceOf(state.byServer, dm.id, dm.peer.id),
  );
  const muted = useChannelMuted(dm.id);
  const badge = useBadge({ channelId: dm.id });
  // The open conversation shows its draft in the composer already.
  const draft = useHasDraft(useUserId(), dm.id) && !active;
  const menu = useNotificationMenu({
    scope: "channel",
    id: dm.id,
    name: dm.peer.name,
    dm: true,
  });
  return (
    <Link
      to="/d/$channelId"
      params={{ channelId: dm.id }}
      aria-current={active ? "page" : undefined}
      tabIndex={tabStop ? 0 : -1}
      className={[
        "dm-row",
        active ? "is-active" : "",
        muted ? "is-muted" : "",
        badge.count > 0 || badge.dot ? "is-unread" : "",
      ].join(" ")}
      {...menu}
      {...ROW_ITEM}
    >
      <PresenceAvatar
        name={dm.peer.name}
        url={dm.peer.avatar_url}
        status={status}
      />
      <span className="dm-row-name">{dm.peer.name}</span>
      {draft ? (
        <span className="channel-row-draft" title="Entwurf">
          <PencilIcon size={13} />
          <span className="sr-only">Entwurf</span>
        </span>
      ) : null}
      <UnreadBadge channelId={dm.id} showDot={false} />
    </Link>
  );
}
