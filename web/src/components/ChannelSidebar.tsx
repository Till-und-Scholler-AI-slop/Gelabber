// Second column: the selected server's categories and channels as one flat,
// virtualised list. Highlight follows the URL param; rows with a `tmp:` id
// are optimistic and not yet clickable.

import { Link, useNavigate } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useMemo, useRef, useState, type ReactNode } from "react";

import { useVoice } from "../voice/session.ts";
import { useVoiceRoster } from "../voice/roster.ts";
import { VoiceControls } from "./VoiceControls.tsx";
import { can } from "../servers/permissions.ts";
import { buildRows, rowHeight, type Row } from "../servers/rows.ts";
import {
  isPendingId,
  useDeleteCategory,
  useDeleteChannel,
} from "../servers/queries.ts";
import type { Category, Channel, ServerDetail } from "../servers/types.ts";
import {
  GearIcon,
  HashIcon,
  LinkIcon,
  PencilIcon,
  PlusIcon,
  SpeakerIcon,
  TrashIcon,
} from "./Icons.tsx";
import { InviteDialog } from "./InviteDialog.tsx";
import {
  CategoryDialog,
  ChannelDialog,
  type CategoryDialogState,
  type ChannelDialogState,
} from "./ServerDialogs.tsx";

export function ChannelSidebar({
  server,
  activeChannelId,
}: {
  server: ServerDetail;
  activeChannelId: string | undefined;
}) {
  const manageChannels = can(server, "manage_channels");
  const manageServer = can(server, "manage_server");
  const canGoLive = can(server, "go_live");
  const voice = useVoice();
  const live = useVoiceRoster((s) => s.live[server.id] ?? {});
  const [channelDialog, setChannelDialog] = useState<ChannelDialogState | null>(
    null,
  );
  const [categoryDialog, setCategoryDialog] =
    useState<CategoryDialogState | null>(null);
  const [inviting, setInviting] = useState(false);

  const rows = useMemo(() => buildRows(server), [server]);

  return (
    <aside
      aria-label="Kanäle"
      className="flex h-full w-64 shrink-0 flex-col border-r border-neutral-200 bg-white"
    >
      <header className="flex h-12 items-center justify-between gap-2 border-b border-neutral-200 px-3">
        <h2
          className="truncate font-semibold tracking-tight"
          title={server.name}
        >
          {server.name}
        </h2>
        <div className="flex shrink-0 items-center gap-0.5">
          <IconButton label="Leute einladen" onClick={() => setInviting(true)}>
            <LinkIcon />
          </IconButton>
          <Link
            to="/s/$serverId/settings"
            params={{ serverId: server.id }}
            title={manageServer ? "Servereinstellungen" : "Mitglieder"}
            aria-label={manageServer ? "Servereinstellungen" : "Mitglieder"}
            className="rounded-md p-1.5 text-neutral-500 transition hover:bg-neutral-100 hover:text-neutral-900"
            activeProps={{ className: "bg-neutral-100 text-neutral-900" }}
          >
            <GearIcon />
          </Link>
        </div>
      </header>

      <ChannelList
        server={server}
        rows={rows}
        activeChannelId={activeChannelId}
        voiceChannelId={voice.channelId}
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

      {voice.status === "joined" ? (
        <div className="flex flex-col gap-1 border-t border-neutral-200 px-3 py-2 text-sm">
          <div className="min-w-0 text-left">
            <p className="text-xs font-medium text-emerald-700">Verbunden</p>
            <p className="truncate text-neutral-700">
              {voice.channelName ?? "Voice"}
            </p>
          </div>
          <VoiceControls compact canGoLive={canGoLive} />
        </div>
      ) : null}

      {manageChannels ? (
        <footer className="flex flex-col gap-1 border-t border-neutral-200 p-2 text-sm">
          <FooterButton
            onClick={() =>
              setChannelDialog({ mode: "create", categoryId: null })
            }
          >
            <PlusIcon size={14} /> Kanal erstellen
          </FooterButton>
          <FooterButton onClick={() => setCategoryDialog({ mode: "create" })}>
            <PlusIcon size={14} /> Kategorie erstellen
          </FooterButton>
        </footer>
      ) : null}

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
  rows: Row[];
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
    estimateSize: (index) => rowHeight(rows[index]),
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 10,
  });
  const deleteChannel = useDeleteChannel(server.id);
  const deleteCategory = useDeleteCategory(server.id);
  const navigate = useNavigate();

  if (rows.length === 0) {
    return (
      <p className="flex-1 px-3 py-4 text-sm text-neutral-500">
        Noch keine Kanäle.
      </p>
    );
  }

  return (
    <div ref={scrollRef} className="flex-1 overflow-y-auto px-2 py-2">
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              className="absolute inset-x-0"
              style={{ top: item.start, height: item.size }}
            >
              {row.kind === "category" ? (
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
                <p className="px-2 text-xs text-neutral-400">Leer</p>
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
        "group flex h-10 items-end justify-between gap-1 px-1 pb-1",
        pending ? "opacity-50" : "",
      ].join(" ")}
    >
      <span className="truncate text-xs font-semibold tracking-wide text-neutral-500 uppercase">
        {category.name}
      </span>
      {manage && !pending ? (
        <span className="flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100">
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
      <Icon size={16} className="shrink-0 text-neutral-400" />
      <span className="truncate">{channel.name}</span>
      {live ? (
        <span className="ml-auto shrink-0 rounded bg-red-600 px-1 py-px text-[10px] font-semibold tracking-wide text-white uppercase">
          Live
        </span>
      ) : inVoice ? (
        <span
          className="ml-auto h-2 w-2 shrink-0 rounded-full bg-emerald-500"
          title="Verbunden"
        />
      ) : null}
    </>
  );
  const rowClass = [
    "group flex h-[34px] items-center gap-1 rounded-md pr-1 text-sm",
    active
      ? "bg-neutral-900 text-white"
      : "text-neutral-700 hover:bg-neutral-100 hover:text-neutral-900",
    pending ? "opacity-50" : "",
  ].join(" ");

  return (
    <div className={rowClass}>
      {pending ? (
        <span className="flex min-w-0 flex-1 items-center gap-1.5 px-2">
          {body}
        </span>
      ) : (
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: channel.server_id, channelId: channel.id }}
          aria-current={active ? "page" : undefined}
          className="flex min-w-0 flex-1 items-center gap-1.5 self-stretch px-2"
        >
          {body}
        </Link>
      )}
      {manage && !pending ? (
        <span
          className={[
            "flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100",
            active ? "text-white" : "",
          ].join(" ")}
        >
          <IconButton
            label="Kanal bearbeiten"
            onClick={onEdit}
            small
            inverted={active}
          >
            <PencilIcon size={14} />
          </IconButton>
          <IconButton
            label="Kanal löschen"
            onClick={onDelete}
            small
            inverted={active}
          >
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
  inverted,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  small?: boolean;
  inverted?: boolean;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={onClick}
      className={[
        "rounded-md transition",
        small ? "p-1" : "p-1.5",
        inverted
          ? "text-neutral-300 hover:bg-white/15 hover:text-white"
          : "text-neutral-500 hover:bg-neutral-200 hover:text-neutral-900",
      ].join(" ")}
    >
      {children}
    </button>
  );
}

function FooterButton({
  onClick,
  children,
}: {
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-neutral-600 transition hover:bg-neutral-100 hover:text-neutral-900"
    >
      {children}
    </button>
  );
}
