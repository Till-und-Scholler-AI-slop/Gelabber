import { UnreadBadge } from "../messages/UnreadBadge.tsx";
// Second column: the selected server's categories and channels as one flat,
// virtualised list. Highlight follows the URL param; rows with a `tmp:` id
// are optimistic and not yet clickable.

import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState, type ReactNode } from "react";

import { useVoice } from "../voice/session.ts";
import { EMPTY_LIVE, useVoiceRoster } from "../voice/roster.ts";
import { can } from "../servers/permissions.ts";
import { buildRows, type Row } from "../servers/rows.ts";
import {
  isPendingId,
  useDeleteCategory,
  useDeleteChannel,
} from "../servers/queries.ts";
import type { Category, Channel, ServerDetail } from "../servers/types.ts";
import {
  ChatIcon,
  GearIcon,
  HashIcon,
  LinkIcon,
  PencilIcon,
  PlusIcon,
  SpeakerIcon,
  TrashIcon,
} from "./Icons.tsx";
import { UserPanel } from "./UserPanel.tsx";
import { InviteDialog } from "./InviteDialog.tsx";
import {
  CategoryDialog,
  ChannelDialog,
  type CategoryDialogState,
  type ChannelDialogState,
} from "./ServerDialogs.tsx";

type SidebarRow = Row | { kind: "section"; key: string; label: string };

// Keep categories intact; give uncategorized voice rooms their own heading.
function sidebarRows(server: ServerDetail): SidebarRow[] {
  const base = buildRows(server);
  const text = base.filter(
    (row) =>
      row.kind === "channel" &&
      row.channel.category_id === null &&
      row.channel.kind === "text",
  );
  const voice = base.filter(
    (row) =>
      row.kind === "channel" &&
      row.channel.category_id === null &&
      row.channel.kind === "voice",
  );
  const categorized = base.filter(
    (row) => row.kind !== "channel" || row.channel.category_id !== null,
  );
  return [
    ...text,
    ...(voice.length
      ? [
          {
            kind: "section" as const,
            key: "voice-heading",
            label: "Sprachkanäle",
          },
          ...voice,
        ]
      : []),
    ...categorized,
  ];
}

function sidebarRowHeight(row: SidebarRow | undefined): number {
  if (row?.kind === "category" || row?.kind === "section") return 34;
  return row?.kind === "empty" ? 28 : 36;
}

export function ChannelSidebar({
  server,
  activeChannelId,
}: {
  server: ServerDetail;
  activeChannelId: string | undefined;
}) {
  const pathname = useRouterState({
    select: (state) => state.location.pathname,
  });
  const manageChannels = can(server, "manage_channels");
  const manageServer = can(server, "manage_server");
  const voice = useVoice();
  const live = useVoiceRoster((s) => s.live[server.id] ?? EMPTY_LIVE);
  const [channelDialog, setChannelDialog] = useState<ChannelDialogState | null>(
    null,
  );
  const [categoryDialog, setCategoryDialog] =
    useState<CategoryDialogState | null>(null);
  const [inviting, setInviting] = useState(false);

  const rows = useMemo(() => sidebarRows(server), [server]);

  return (
    <aside aria-label="Kanäle" className="channel-sidebar">
      <header className="server-sidebar-heading">
        <h2 className="server-sidebar-name" title={server.name}>
          {server.name}
        </h2>
        <div className="server-sidebar-actions">
          <IconButton label="Leute einladen" onClick={() => setInviting(true)}>
            <LinkIcon />
          </IconButton>
          {manageChannels ? (
            <details className="sidebar-create-menu">
              <summary
                className="shell-icon-button"
                aria-label="Kanal oder Kategorie erstellen"
                title="Erstellen"
              >
                <PlusIcon size={17} />
              </summary>
              <nav aria-label="Erstellen">
                <button
                  type="button"
                  onClick={(event) => {
                    event.currentTarget
                      .closest("details")
                      ?.removeAttribute("open");
                    setChannelDialog({ mode: "create", categoryId: null });
                  }}
                >
                  Kanal erstellen
                </button>
                <button
                  type="button"
                  onClick={(event) => {
                    event.currentTarget
                      .closest("details")
                      ?.removeAttribute("open");
                    setCategoryDialog({ mode: "create" });
                  }}
                >
                  Kategorie erstellen
                </button>
              </nav>
            </details>
          ) : null}
          <Link
            to="/s/$serverId/settings"
            params={{ serverId: server.id }}
            title={manageServer ? "Servereinstellungen" : "Mitglieder"}
            aria-label={manageServer ? "Servereinstellungen" : "Mitglieder"}
            className="shell-icon-button"
            activeProps={{ className: "is-active" }}
          >
            <GearIcon />
          </Link>
        </div>
      </header>

      <Link
        to="/s/$serverId"
        params={{ serverId: server.id }}
        className={`sidebar-overview ${pathname === `/s/${server.id}` ? "is-active" : ""}`}
        aria-current={pathname === `/s/${server.id}` ? "page" : undefined}
        activeOptions={{ exact: true }}
      >
        <ChatIcon size={20} />
        Übersicht
      </Link>
      <ChannelList
        server={server}
        rows={rows}
        activeChannelId={activeChannelId}
        voiceChannelId={voice.serverId === server.id ? voice.channelId : null}
        liveChannels={live}
        manageChannels={manageChannels}
        onEditChannel={(channel) => setChannelDialog({ mode: "edit", channel })}
        onCreateChannel={(categoryId) =>
          setChannelDialog({ mode: "create", categoryId })
        }
        onEditCategory={(category) =>
          setCategoryDialog({ mode: "edit", category })
        }
      />

      <UserPanel contextId={server.id} />
      <ChannelDialog
        server={server}
        state={channelDialog}
        onClose={() => setChannelDialog(null)}
      />
      <CategoryDialog
        server={server}
        state={categoryDialog}
        onClose={() => setCategoryDialog(null)}
      />
      <InviteDialog
        server={server}
        open={inviting}
        onClose={() => setInviting(false)}
      />
    </aside>
  );
}

function ChannelList({
  server,
  rows,
  activeChannelId,
  voiceChannelId,
  liveChannels,
  manageChannels,
  onEditChannel,
  onCreateChannel,
  onEditCategory,
}: {
  server: ServerDetail;
  rows: SidebarRow[];
  activeChannelId: string | undefined;
  voiceChannelId: string | null;
  liveChannels: Record<string, string>;
  manageChannels: boolean;
  onEditChannel: (channel: Channel) => void;
  onCreateChannel: (categoryId: string | null) => void;
  onEditCategory: (category: Category) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => sidebarRowHeight(rows[index]),
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 10,
  });
  const deleteChannel = useDeleteChannel(server.id);
  const deleteCategory = useDeleteCategory(server.id);
  const navigate = useNavigate();

  if (rows.length === 0) {
    return <p className="sidebar-empty">Noch keine Kanäle.</p>;
  }

  return (
    <div ref={scrollRef} className="sidebar-list-scroll">
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              className="sidebar-virtual-row"
              style={{ top: item.start, height: item.size }}
            >
              {row.kind === "section" ? (
                <p className="sidebar-section-heading">{row.label}</p>
              ) : row.kind === "category" ? (
                <CategoryRow
                  category={row.category}
                  manage={manageChannels}
                  onAdd={() => onCreateChannel(row.category.id)}
                  onEdit={() => onEditCategory(row.category)}
                  onDelete={() => {
                    if (
                      window.confirm(
                        `Kategorie „${row.category.name}“ löschen? Die Kanäle bleiben erhalten.`,
                      )
                    )
                      deleteCategory.mutate(row.category.id);
                  }}
                />
              ) : row.kind === "channel" ? (
                <ChannelRow
                  channel={row.channel}
                  active={row.channel.id === activeChannelId}
                  inVoice={row.channel.id === voiceChannelId}
                  live={Boolean(liveChannels[row.channel.id])}
                  manage={manageChannels}
                  onEdit={() => onEditChannel(row.channel)}
                  onDelete={() => {
                    if (!window.confirm(`Kanal „${row.channel.name}“ löschen?`))
                      return;
                    deleteChannel.mutate(row.channel.id);
                    if (row.channel.id === activeChannelId)
                      void navigate({
                        to: "/s/$serverId",
                        params: { serverId: server.id },
                        replace: true,
                      });
                  }}
                />
              ) : (
                <p className="sidebar-empty-category">Leer</p>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function CategoryRow({
  category,
  manage,
  onAdd,
  onEdit,
  onDelete,
}: {
  category: Category;
  manage: boolean;
  onAdd: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(category.id);
  return (
    <div
      className={[
        "group sidebar-category-row",
        pending ? "opacity-50" : "",
      ].join(" ")}
    >
      <span className="sidebar-category-name">{category.name}</span>
      {manage && !pending ? (
        <span className="sidebar-row-actions flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100">
          <IconButton label="Kanal in dieser Kategorie" onClick={onAdd} small>
            <PlusIcon size={14} />
          </IconButton>
          <IconButton label="Kategorie umbenennen" onClick={onEdit} small>
            <PencilIcon size={14} />
          </IconButton>
          <IconButton label="Kategorie löschen" onClick={onDelete} small>
            <TrashIcon size={14} />
          </IconButton>
        </span>
      ) : null}
    </div>
  );
}

function ChannelRow({
  channel,
  active,
  inVoice,
  live,
  manage,
  onEdit,
  onDelete,
}: {
  channel: Channel;
  active: boolean;
  inVoice: boolean;
  live: boolean;
  manage: boolean;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(channel.id);
  const Icon = channel.kind === "voice" ? SpeakerIcon : HashIcon;
  const body = (
    <>
      <Icon size={25} className="channel-row-icon" />
      <span className="channel-row-name">{channel.name}</span>
      {channel.kind === "text" ? <UnreadBadge channelId={channel.id} /> : null}
      {live ? (
        <span className="channel-row-live">Live</span>
      ) : inVoice ? (
        <span className="channel-row-connected" title="Verbunden" />
      ) : null}
    </>
  );
  const rowClass = [
    "group channel-row",
    active ? "is-active" : "",
    pending ? "opacity-50" : "",
  ].join(" ");

  return (
    <div className={rowClass}>
      {pending ? (
        <span className="channel-row-link">{body}</span>
      ) : (
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: channel.server_id, channelId: channel.id }}
          aria-current={active ? "page" : undefined}
          className="channel-row-link"
        >
          {body}
        </Link>
      )}
      {manage && !pending ? (
        <span
          className={[
            "sidebar-row-actions flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100",
          ].join(" ")}
        >
          <IconButton label="Kanal bearbeiten" onClick={onEdit} small>
            <PencilIcon size={14} />
          </IconButton>
          <IconButton label="Kanal löschen" onClick={onDelete} small>
            <TrashIcon size={14} />
          </IconButton>
        </span>
      ) : null}
    </div>
  );
}

function IconButton({
  label,
  onClick,
  children,
  small,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  small?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={[
        "shell-icon-button",
        small ? "sidebar-action-small" : "",
      ].join(" ")}
    >
      {children}
    </button>
  );
}
