import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import {
  notificationMenu,
  openNotificationSettings,
} from "./notificationTarget.ts";
import {
  useChannelMuted,
  useServerMuted,
} from "../messages/notificationSettings.ts";
// Second column: the selected server's categories and channels as one flat,
// virtualised list. Highlight follows the URL param; rows with a `tmp:` id
// are optimistic and not yet clickable.

import { Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import { useSession } from "../auth/session.ts";
import { useVoice } from "../voice/session.ts";
import { useSpeaking } from "../voice/speaking.ts";
import {
  EMPTY_LIVE,
  EMPTY_OCCUPANCY,
  useVoiceRoster,
  type VoiceFlags,
} from "../voice/roster.ts";
import { can } from "../servers/permissions.ts";
import { useMenuDetails } from "./menuDetails.ts";
import { focusRowNear, ROW_ITEM, useRovingRow } from "./rowKeys.ts";
import { buildRows, type Row } from "../servers/rows.ts";
import {
  isPendingId,
  useDeleteCategory,
  useDeleteChannel,
} from "../servers/queries.ts";
import type {
  Category,
  Channel,
  Member,
  ServerDetail,
} from "../servers/types.ts";
import { Avatar } from "./Avatar.tsx";
import {
  BellIcon,
  BellOffIcon,
  ChatIcon,
  GearIcon,
  HashIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicOffIcon,
  LinkIcon,
  PencilIcon,
  PlusIcon,
  SpeakerIcon,
  TrashIcon,
} from "./Icons.tsx";
import { UserPanel } from "./UserPanel.tsx";
import { MemberProfileDialog } from "./MemberProfileDialog.tsx";
import { LocallyMutedIcon } from "./UserVolumeControl.tsx";
import { InviteDialog } from "./InviteDialog.tsx";
import { confirmAction } from "./confirm.ts";
import {
  CategoryDialog,
  ChannelDialog,
  type CategoryDialogState,
  type ChannelDialogState,
} from "./ServerDialogs.tsx";

type ParticipantRow = {
  kind: "participant";
  key: string;
  member: Pick<Member, "user_id" | "name" | "avatar_url">;
  flags: VoiceFlags;
  live: boolean;
};

type SidebarRow =
  Row | { kind: "section"; key: string; label: string } | ParticipantRow;

/** Who sits in each voice channel, listed right below it. */
function withParticipants(
  rows: SidebarRow[],
  server: ServerDetail,
  occupancy: Record<string, VoiceFlags>,
  live: Record<string, string>,
): SidebarRow[] {
  const members = new Map(server.members.map((m) => [m.user_id, m]));
  const byChannel = new Map<string, ParticipantRow[]>();
  for (const [userId, flags] of Object.entries(occupancy)) {
    const member = members.get(userId) ?? {
      user_id: userId,
      name: "Jemand",
      avatar_url: null,
    };
    const list = byChannel.get(flags.channelId) ?? [];
    list.push({
      kind: "participant",
      key: `participant:${flags.channelId}:${userId}`,
      member,
      flags,
      live: live[flags.channelId] === userId,
    });
    byChannel.set(flags.channelId, list);
  }
  if (byChannel.size === 0) return rows;
  return rows.flatMap((row) => {
    if (row.kind !== "channel" || row.channel.kind !== "voice") return [row];
    const inside = byChannel.get(row.channel.id);
    if (!inside) return [row];
    inside.sort((a, b) => a.member.name.localeCompare(b.member.name, "de"));
    return [row, ...inside];
  });
}

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

// Touch screens get finger-sized channel and category rows; index.css sizes
// their content to match under the same media query.
const TOUCH_ROWS = "(pointer: coarse)";

function subscribeTouchRows(notify: () => void) {
  const media = window.matchMedia(TOUCH_ROWS);
  media.addEventListener("change", notify);
  return () => media.removeEventListener("change", notify);
}

function sidebarRowHeight(row: SidebarRow | undefined, touch: boolean): number {
  if (row?.kind === "section") return 34;
  if (row?.kind === "category") return touch ? 44 : 34;
  if (row?.kind === "participant") return 28;
  if (row?.kind === "empty") return 28;
  return touch ? 46 : 36;
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
  const voiceChannelId = useVoice((s) =>
    s.serverId === server.id ? s.channelId : null,
  );
  const live = useVoiceRoster((s) => s.live[server.id] ?? EMPTY_LIVE);
  const occupancy = useVoiceRoster(
    (s) => s.byServer[server.id] ?? EMPTY_OCCUPANCY,
  );
  const rows = useMemo(
    () => withParticipants(sidebarRows(server), server, occupancy, live),
    [server, occupancy, live],
  );
  const [channelDialog, setChannelDialog] = useState<ChannelDialogState | null>(
    null,
  );
  const [categoryDialog, setCategoryDialog] =
    useState<CategoryDialogState | null>(null);
  const [inviting, setInviting] = useState(false);
  const [profileId, setProfileId] = useState<string | null>(null);
  const createMenu = useMenuDetails();
  const serverMuted = useServerMuted(server.id);

  return (
    <aside aria-label="Kanäle" className="channel-sidebar">
      <header className="server-sidebar-heading">
        <h2 className="server-sidebar-name" title={server.name}>
          {server.name}
        </h2>
        <div className="server-sidebar-actions">
          <IconButton
            label={`Benachrichtigungen für ${server.name}${serverMuted ? " (stumm)" : ""}`}
            onClick={() =>
              openNotificationSettings({
                scope: "server",
                id: server.id,
                name: server.name,
              })
            }
          >
            {serverMuted ? <BellOffIcon /> : <BellIcon />}
          </IconButton>
          <IconButton label="Leute einladen" onClick={() => setInviting(true)}>
            <LinkIcon />
          </IconButton>
          {manageChannels ? (
            <details ref={createMenu} className="sidebar-create-menu">
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
                  onClick={() => {
                    setChannelDialog({ mode: "create", categoryId: null });
                  }}
                >
                  Kanal erstellen
                </button>
                <button
                  type="button"
                  onClick={() => {
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
        voiceChannelId={voiceChannelId}
        liveChannels={live}
        manageChannels={manageChannels}
        onEditChannel={(channel) => setChannelDialog({ mode: "edit", channel })}
        onCreateChannel={(categoryId) =>
          setChannelDialog({ mode: "create", categoryId })
        }
        onEditCategory={(category) =>
          setCategoryDialog({ mode: "edit", category })
        }
        onOpenProfile={setProfileId}
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
      {profileId ? (
        <MemberProfileDialog
          server={server}
          memberId={profileId}
          onClose={() => setProfileId(null)}
        />
      ) : null}
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
  onOpenProfile,
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
  onOpenProfile: (memberId: string) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const touch = useSyncExternalStore(
    subscribeTouchRows,
    () => window.matchMedia(TOUCH_ROWS).matches,
    () => false,
  );
  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => sidebarRowHeight(rows[index], touch),
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan: 10,
  });
  // Cached row positions still use the previous pitch.
  useEffect(() => virtualizer.measure(), [touch, virtualizer]);
  const deleteChannel = useDeleteChannel(server.id);
  const deleteCategory = useDeleteCategory(server.id);
  const navigate = useNavigate();
  const rowHintId = useId();
  // The key hint is read once, on the list's first row with actions, not
  // on every Tab stop.
  const hintKey = manageChannels
    ? rows.find((row) => row.kind === "category" || row.kind === "channel")?.key
    : undefined;
  // The row that last held focus. When it is deleted (from here or by
  // someone else) focus would fall to the page; it goes to the row now in
  // its place instead, unless the user has moved on.
  const lastFocused = useRef<{ key: string; index: number } | null>(null);
  useEffect(() => {
    const last = lastFocused.current;
    if (!last || rows.some((row) => row.key === last.key)) return;
    lastFocused.current = null;
    // After the commit's other effects, e.g. a confirm dialog closing and
    // handing focus back to the deleted row's button.
    window.setTimeout(() => {
      const active = document.activeElement;
      if (active && active !== document.body) return;
      if (scrollRef.current) focusRowNear(scrollRef.current, last.index);
    }, 0);
  }, [rows]);

  if (rows.length === 0) {
    return <p className="sidebar-empty">Noch keine Kanäle.</p>;
  }

  return (
    <div ref={scrollRef} className="sidebar-list-scroll">
      {manageChannels ? (
        <p id={rowHintId} className="sr-only">
          Pfeiltasten links und rechts wechseln zwischen dem Eintrag und seinen
          Aktionen, Pos1 und Ende springen an den Anfang und das Ende der Zeile.
        </p>
      ) : null}
      <div style={{ height: virtualizer.getTotalSize() }} className="relative">
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <div
              key={item.key}
              data-index={item.index}
              className="sidebar-virtual-row"
              onFocus={() => {
                lastFocused.current = { key: row.key, index: item.index };
              }}
              style={{ top: item.start, height: item.size }}
            >
              {row.kind === "section" ? (
                <p className="sidebar-section-heading">{row.label}</p>
              ) : row.kind === "category" ? (
                <CategoryRow
                  category={row.category}
                  manage={manageChannels}
                  hintId={row.key === hintKey ? rowHintId : undefined}
                  onAdd={() => onCreateChannel(row.category.id)}
                  onEdit={() => onEditCategory(row.category)}
                  onDelete={async () => {
                    if (
                      await confirmAction({
                        title: `Kategorie „${row.category.name}“ löschen?`,
                        message: "Die Kanäle darin bleiben erhalten.",
                        confirmLabel: "Kategorie löschen",
                      })
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
                  hintId={row.key === hintKey ? rowHintId : undefined}
                  onEdit={() => onEditChannel(row.channel)}
                  onDelete={async () => {
                    if (
                      !(await confirmAction({
                        title: `Kanal „${row.channel.name}“ löschen?`,
                        message:
                          row.channel.kind === "text"
                            ? "Alle Nachrichten darin werden gelöscht."
                            : undefined,
                        confirmLabel: "Kanal löschen",
                      }))
                    )
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
              ) : row.kind === "participant" ? (
                <ParticipantRowView
                  row={row}
                  serverId={server.id}
                  onOpenProfile={onOpenProfile}
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

export function ParticipantRowView({
  row,
  serverId,
  onOpenProfile,
}: {
  row: ParticipantRow;
  serverId: string;
  onOpenProfile: (memberId: string) => void;
}) {
  const me = useSession((s) => s.user?.id);
  const self = row.member.user_id === me;
  const { member, flags, live } = row;
  const state = flags.deafened ? "taub" : flags.muted ? "stumm" : null;
  const speaking =
    useSpeaking(serverId, flags.channelId, member.user_id) && state === null;
  return (
    // A button, so a tap (iOS sends no contextmenu on long-press) and the
    // keyboard reach the profile with its volume controls too.
    <button
      type="button"
      className={`sidebar-participant${speaking ? " is-speaking" : ""}`}
      title={self ? "Dein Profil" : "Profil und Lautstärke"}
      onClick={() => onOpenProfile(member.user_id)}
      onContextMenu={(event) => {
        event.preventDefault();
        onOpenProfile(member.user_id);
      }}
    >
      <Avatar
        name={member.name}
        url={member.avatar_url}
        className={speaking ? "is-speaking" : ""}
      />
      <span className="sidebar-participant-name">{member.name}</span>
      {live ? (
        <span className="sidebar-participant-live" title="Live">
          <LiveIcon size={13} />
          <span className="sr-only">Live</span>
        </span>
      ) : null}
      {self ? null : <LocallyMutedIcon userId={row.member.user_id} />}
      {state ? (
        <span className="sidebar-participant-state" title={state}>
          {flags.deafened ? (
            <HeadsetOffIcon size={13} />
          ) : (
            <MicOffIcon size={13} />
          )}
          <span className="sr-only">{state}</span>
        </span>
      ) : null}
    </button>
  );
}

function CategoryRow({
  category,
  manage,
  hintId,
  onAdd,
  onEdit,
  onDelete,
}: {
  category: Category;
  manage: boolean;
  hintId: string | undefined;
  onAdd: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(category.id);
  const roving = useRovingRow(manage && !pending);
  return (
    <div
      className={[
        "group sidebar-category-row",
        pending ? "opacity-50" : "",
      ].join(" ")}
      {...roving.keys}
    >
      <span className="sidebar-category-name">{category.name}</span>
      {manage && !pending ? (
        <span className="sidebar-row-actions flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100">
          <IconButton
            label="Kanal in dieser Kategorie"
            onClick={onAdd}
            small
            describedBy={hintId}
            tabIndex={roving.tabIndex(0)}
          >
            <PlusIcon size={14} />
          </IconButton>
          <IconButton
            label="Kategorie umbenennen"
            onClick={onEdit}
            small
            tabIndex={roving.tabIndex(1)}
          >
            <PencilIcon size={14} />
          </IconButton>
          <IconButton
            label="Kategorie löschen"
            onClick={onDelete}
            small
            tabIndex={roving.tabIndex(2)}
          >
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
  hintId,
  onEdit,
  onDelete,
}: {
  channel: Channel;
  active: boolean;
  inVoice: boolean;
  live: boolean;
  manage: boolean;
  hintId: string | undefined;
  onEdit: () => void;
  onDelete: () => void;
}) {
  const pending = isPendingId(channel.id);
  const muted = useChannelMuted(channel.id);
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
    muted && channel.kind === "text" ? "is-muted" : "",
  ].join(" ");
  const roving = useRovingRow(manage && !pending);

  return (
    <div className={rowClass} {...roving.keys}>
      {pending ? (
        <span className="channel-row-link">{body}</span>
      ) : (
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: channel.server_id, channelId: channel.id }}
          aria-current={active ? "page" : undefined}
          aria-describedby={manage ? hintId : undefined}
          tabIndex={roving.tabIndex(0)}
          className="channel-row-link"
          onContextMenu={
            channel.kind === "text"
              ? notificationMenu({
                  scope: "channel",
                  id: channel.id,
                  name: `#${channel.name}`,
                  serverId: channel.server_id,
                })
              : undefined
          }
          {...ROW_ITEM}
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
          <IconButton
            label="Kanal bearbeiten"
            onClick={onEdit}
            small
            tabIndex={roving.tabIndex(1)}
          >
            <PencilIcon size={14} />
          </IconButton>
          <IconButton
            label="Kanal löschen"
            onClick={onDelete}
            small
            tabIndex={roving.tabIndex(2)}
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
  tabIndex,
  describedBy,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  small?: boolean;
  /** -1 for a row item that is not its row's Tab stop right now. */
  tabIndex?: number;
  describedBy?: string;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-describedby={describedBy}
      tabIndex={tabIndex}
      onClick={onClick}
      {...ROW_ITEM}
      className={[
        "shell-icon-button",
        small ? "sidebar-action-small" : "",
      ].join(" ")}
    >
      {children}
    </button>
  );
}
