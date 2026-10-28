import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadge } from "../messages/useBadge.ts";
import { openNotificationSettings } from "./notificationTarget.ts";
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
  type MouseEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";

import { useUserId } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { useHasDraft } from "../messages/drafts.ts";
import { useVoice } from "../voice/session.ts";
import { useSpeaking } from "../voice/speaking.ts";
import {
  EMPTY_LIVE,
  EMPTY_LIVE_PEOPLE,
  EMPTY_OCCUPANCY,
  useVoiceRoster,
  type LiveMap,
  type VoiceFlags,
} from "../voice/roster.ts";
import { liveSentence } from "../voice/roomSummary.ts";
import { can } from "../servers/permissions.ts";
import { useHoldMenu } from "./useHoldMenu.ts";
import { useMenuDetails } from "./menuDetails.ts";
import { useNavigationDrawer } from "./navigationDrawer.ts";
import {
  focusRowNear,
  LIST_ROW,
  listStop,
  moveWithinList,
  ROW_ITEM,
  useRovingRow,
} from "./rowKeys.ts";
import { sidebarRows, type Row, type SectionRow } from "../servers/rows.ts";
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
  ChevronIcon,
  GearIcon,
  HashIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicOffIcon,
  LinkIcon,
  MoreIcon,
  PencilIcon,
  PlusIcon,
  SpeakerIcon,
  TrashIcon,
  UsersIcon,
} from "./Icons.tsx";
import { Popover } from "./ui/Popover.tsx";
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
import "./shell.css";

type ParticipantRow = {
  kind: "participant";
  key: string;
  member: Pick<Member, "user_id" | "name" | "avatar_url">;
  flags: VoiceFlags;
  live: boolean;
};

type SidebarRow = Row | SectionRow | ParticipantRow;

/** Who sits in each voice channel, listed right below it. */
function withParticipants(
  rows: SidebarRow[],
  server: ServerDetail,
  occupancy: Record<string, VoiceFlags>,
  live: LiveMap,
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
      live: Boolean(live[flags.channelId]?.[userId]),
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
  if (row?.kind === "participant") return touch ? 40 : 28;
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
  const serverMenu = useMenuDetails();
  const serverMuted = useServerMuted(server.id);
  const drawer = useNavigationDrawer();
  // On a phone these open from the drawer: it goes first, so the dialog is
  // not stacked on top of it and gives focus back to the drawer's button.
  const fromDrawer = (open: () => void) => {
    if (drawer.open) void drawer.hide().then(open);
    else open();
  };
  const invite = () => fromDrawer(() => setInviting(true));
  // A new channel goes where the user is: the category of the channel open.
  const viewedCategory =
    server.channels.find((channel) => channel.id === activeChannelId)
      ?.category_id ?? null;

  return (
    <aside aria-label="Kanäle" className="channel-sidebar">
      <header className="server-sidebar-heading">
        <details ref={serverMenu} className="server-sidebar-menu">
          <summary className="server-sidebar-menu-button" title={server.name}>
            <h2 className="server-sidebar-name">{server.name}</h2>
            {serverMuted ? (
              <span className="server-sidebar-muted" title="Stummgeschaltet">
                <BellOffIcon size={15} />
                <span className="sr-only">(stummgeschaltet)</span>
              </span>
            ) : null}
            <ChevronIcon size={16} className="server-sidebar-chevron" />
          </summary>
          <nav aria-label={`Menü für ${server.name}`} className="shell-menu">
            <button type="button" className="shell-menu-item" onClick={invite}>
              <LinkIcon size={16} />
              Leute einladen
            </button>
            <button
              type="button"
              className="shell-menu-item"
              onClick={() =>
                fromDrawer(() =>
                  openNotificationSettings({
                    scope: "server",
                    id: server.id,
                    name: server.name,
                  }),
                )
              }
            >
              {serverMuted ? <BellOffIcon size={16} /> : <BellIcon size={16} />}
              Benachrichtigungen…
            </button>
            {manageChannels ? (
              <>
                <button
                  type="button"
                  className="shell-menu-item"
                  onClick={() =>
                    fromDrawer(() =>
                      setChannelDialog({
                        mode: "create",
                        categoryId: viewedCategory,
                      }),
                    )
                  }
                >
                  <PlusIcon size={16} />
                  Kanal erstellen
                </button>
                <button
                  type="button"
                  className="shell-menu-item"
                  onClick={() =>
                    fromDrawer(() => setCategoryDialog({ mode: "create" }))
                  }
                >
                  <PlusIcon size={16} />
                  Kategorie erstellen
                </button>
              </>
            ) : null}
            <Link
              to="/s/$serverId/settings"
              params={{ serverId: server.id }}
              className="shell-menu-item"
            >
              {manageServer ? <GearIcon size={16} /> : <UsersIcon size={16} />}
              {manageServer ? "Servereinstellungen" : "Mitglieder"}
            </Link>
          </nav>
        </details>
        <IconButton label="Leute einladen" onClick={invite}>
          <LinkIcon />
        </IconButton>
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
        onEditChannel={(channel) =>
          fromDrawer(() => setChannelDialog({ mode: "edit", channel }))
        }
        onCreateChannel={(categoryId) =>
          fromDrawer(() => setChannelDialog({ mode: "create", categoryId }))
        }
        onEditCategory={(category) =>
          fromDrawer(() => setCategoryDialog({ mode: "edit", category }))
        }
        onOpenProfile={(memberId) => fromDrawer(() => setProfileId(memberId))}
        onChannelNotifications={(channel) =>
          fromDrawer(() =>
            openNotificationSettings({
              scope: "channel",
              id: channel.id,
              name: `#${channel.name}`,
              serverId: channel.server_id,
            }),
          )
        }
      />

      <UserPanel contextId={server.id} />
      {/* Outside the drawer, which closes as they open. */}
      {typeof document === "undefined"
        ? null
        : createPortal(
            <>
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
            </>,
            document.body,
          )}
    </aside>
  );
}

type RowMenu =
  | { kind: "channel"; channel: Channel }
  | { kind: "category"; category: Category };

/** Rows the arrow keys stop at: those with a link or a button. */
function navigable(row: SidebarRow, manageChannels: boolean): boolean {
  if (row.kind === "participant") return true;
  if (row.kind === "channel") return !isPendingId(row.channel.id);
  if (row.kind === "category")
    return manageChannels && !isPendingId(row.category.id);
  return false;
}

export function ChannelList({
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
  onChannelNotifications,
}: {
  server: ServerDetail;
  rows: SidebarRow[];
  activeChannelId: string | undefined;
  voiceChannelId: string | null;
  liveChannels: LiveMap;
  manageChannels: boolean;
  onEditChannel: (channel: Channel) => void;
  onCreateChannel: (categoryId: string | null) => void;
  onEditCategory: (category: Category) => void;
  onOpenProfile: (memberId: string) => void;
  onChannelNotifications: (channel: Channel) => void;
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
    // The first rows render before the list is measured.
    initialRect: { width: 0, height: 600 },
  });
  // Cached row positions still use the previous pitch.
  useEffect(() => virtualizer.measure(), [touch, virtualizer]);
  const deleteChannel = useDeleteChannel(server.id);
  const deleteCategory = useDeleteCategory(server.id);
  const navigate = useNavigate();
  const rowHintId = useId();
  const [menu, setMenu] = useState<RowMenu | null>(null);
  const menuAnchor = useRef<HTMLElement | null>(null);
  const openMenu = (anchor: HTMLElement, next: RowMenu) => {
    menuAnchor.current = anchor;
    setMenu(next);
  };
  // What a held finger opens: the row menu, or for members a text
  // channel's notifications.
  const holdHint = manageChannels
    ? "Gedrückt halten für Bearbeiten, Benachrichtigungen oder Löschen."
    : rows.some((row) => row.kind === "channel" && row.channel.kind === "text")
      ? "Gedrückt halten für Benachrichtigungen."
      : null;
  // The key hint is read once, on the list's first row, not on every Tab
  // stop.
  const hintKey = rows.find((row) => navigable(row, manageChannels))?.key;
  // The list is one Tab stop; ↑ and ↓ move between its rows.
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const items = virtualizer.getVirtualItems();
  const stop = listStop(
    items.flatMap((item) => {
      const row = rows[item.index];
      return row && navigable(row, manageChannels) ? [row.key] : [];
    }),
    focusedKey,
    activeChannelId,
  );
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

  const removeChannel = async (channel: Channel) => {
    if (
      !(await confirmAction({
        title: `Kanal „${channel.name}“ löschen?`,
        message:
          channel.kind === "text"
            ? "Alle Nachrichten darin werden gelöscht."
            : undefined,
        confirmLabel: "Kanal löschen",
      }))
    )
      return;
    deleteChannel.mutate(channel.id);
    if (channel.id === activeChannelId)
      void navigate({
        to: "/s/$serverId",
        params: { serverId: server.id },
        replace: true,
      });
  };
  const removeCategory = async (category: Category) => {
    if (
      await confirmAction({
        title: `Kategorie „${category.name}“ löschen?`,
        message: "Die Kanäle darin bleiben erhalten.",
        confirmLabel: "Kategorie löschen",
      })
    )
      deleteCategory.mutate(category.id);
  };

  return (
    <div ref={scrollRef} className="sidebar-list-scroll">
      <p id={rowHintId} className="sr-only">
        {manageChannels
          ? "Pfeiltasten hoch und runter wechseln zwischen den Einträgen, links und rechts zwischen einem Eintrag und seinen Aktionen. Pos1 und Ende springen an den Anfang und das Ende der Zeile."
          : "Pfeiltasten hoch und runter wechseln zwischen den Einträgen."}
      </p>
      <div
        role="list"
        aria-label="Kanäle"
        style={{ height: virtualizer.getTotalSize() }}
        className="relative"
        onKeyDown={moveWithinList}
      >
        {items.map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          const hintId = row.key === hintKey ? rowHintId : undefined;
          return (
            <div
              key={item.key}
              data-index={item.index}
              role="listitem"
              aria-setsize={rows.length}
              aria-posinset={item.index + 1}
              className="sidebar-virtual-row"
              onFocus={() => {
                lastFocused.current = { key: row.key, index: item.index };
                setFocusedKey(row.key);
              }}
              style={{ top: item.start, height: item.size }}
              {...(navigable(row, manageChannels) ? LIST_ROW : {})}
            >
              {row.kind === "section" ? (
                <h3 className="sidebar-section-heading">{row.label}</h3>
              ) : row.kind === "category" ? (
                <CategoryRow
                  category={row.category}
                  manage={manageChannels}
                  coarse={touch}
                  stop={row.key === stop}
                  hintId={hintId}
                  onAdd={() => onCreateChannel(row.category.id)}
                  onEdit={() => onEditCategory(row.category)}
                  onDelete={() => void removeCategory(row.category)}
                  onMenu={(anchor) =>
                    openMenu(anchor, {
                      kind: "category",
                      category: row.category,
                    })
                  }
                />
              ) : row.kind === "channel" ? (
                <ChannelRow
                  channel={row.channel}
                  active={row.channel.id === activeChannelId}
                  inVoice={row.channel.id === voiceChannelId}
                  live={Object.keys(
                    liveChannels[row.channel.id] ?? EMPTY_LIVE_PEOPLE,
                  ).map(
                    (id) =>
                      server.members.find((member) => member.user_id === id)
                        ?.name ?? "Jemand",
                  )}
                  manage={manageChannels}
                  coarse={touch}
                  stop={row.key === stop}
                  hintId={hintId}
                  onEdit={() => onEditChannel(row.channel)}
                  onDelete={() => void removeChannel(row.channel)}
                  onMenu={(anchor) =>
                    openMenu(anchor, { kind: "channel", channel: row.channel })
                  }
                />
              ) : row.kind === "participant" ? (
                <ParticipantRowView
                  row={row}
                  serverId={server.id}
                  stop={row.key === stop}
                  hintId={hintId}
                  onOpenProfile={onOpenProfile}
                />
              ) : (
                <p className="sidebar-empty-category">Leer</p>
              )}
            </div>
          );
        })}
      </div>
      {touch && holdHint ? (
        // Screen readers have each row's actions button instead.
        <p className="sidebar-hold-hint" aria-hidden="true">
          {holdHint}
        </p>
      ) : null}
      <RowMenuPopover
        menu={menu}
        anchor={menuAnchor}
        onClose={() => setMenu(null)}
        onAdd={(category) => onCreateChannel(category.id)}
        onEditCategory={onEditCategory}
        onDeleteCategory={(category) => void removeCategory(category)}
        onEditChannel={onEditChannel}
        onDeleteChannel={(channel) => void removeChannel(channel)}
        onNotifications={onChannelNotifications}
      />
    </div>
  );
}

/**
 * A row's actions in one place: on touch screens from a long press, with a
 * mouse from a right click. Deleting comes last.
 */
function RowMenuPopover({
  menu,
  anchor,
  onClose,
  onAdd,
  onEditCategory,
  onDeleteCategory,
  onEditChannel,
  onDeleteChannel,
  onNotifications,
}: {
  menu: RowMenu | null;
  anchor: RefObject<HTMLElement | null>;
  onClose: () => void;
  onAdd: (category: Category) => void;
  onEditCategory: (category: Category) => void;
  onDeleteCategory: (category: Category) => void;
  onEditChannel: (channel: Channel) => void;
  onDeleteChannel: (channel: Channel) => void;
  onNotifications: (channel: Channel) => void;
}) {
  const choose = (action: () => void) => () => {
    onClose();
    action();
  };
  if (!menu) return null;
  const label =
    menu.kind === "category"
      ? `Kategorie ${menu.category.name}`
      : menu.channel.kind === "text"
        ? `#${menu.channel.name}`
        : menu.channel.name;
  return (
    <Popover
      open
      onClose={onClose}
      anchor={anchor}
      label={label}
      className="shell-row-menu"
    >
      {menu.kind === "category" ? (
        <>
          <button
            type="button"
            className="shell-menu-item"
            onClick={choose(() => onAdd(menu.category))}
          >
            <PlusIcon size={18} />
            Kanal in dieser Kategorie
          </button>
          <button
            type="button"
            className="shell-menu-item"
            onClick={choose(() => onEditCategory(menu.category))}
          >
            <PencilIcon size={18} />
            Kategorie umbenennen
          </button>
          <hr className="shell-menu-separator" />
          <button
            type="button"
            className="shell-menu-item is-danger"
            onClick={choose(() => onDeleteCategory(menu.category))}
          >
            <TrashIcon size={18} />
            Kategorie löschen
          </button>
        </>
      ) : (
        <>
          <button
            type="button"
            className="shell-menu-item"
            onClick={choose(() => onEditChannel(menu.channel))}
          >
            <PencilIcon size={18} />
            Kanal bearbeiten
          </button>
          {menu.channel.kind === "text" ? (
            <button
              type="button"
              className="shell-menu-item"
              onClick={choose(() => onNotifications(menu.channel))}
            >
              <BellIcon size={18} />
              Benachrichtigungen…
            </button>
          ) : null}
          <hr className="shell-menu-separator" />
          <button
            type="button"
            className="shell-menu-item is-danger"
            onClick={choose(() => onDeleteChannel(menu.channel))}
          >
            <TrashIcon size={18} />
            Kanal löschen
          </button>
        </>
      )}
    </Popover>
  );
}

export function ParticipantRowView({
  row,
  serverId,
  stop = true,
  hintId,
  onOpenProfile,
}: {
  row: ParticipantRow;
  serverId: string;
  /** Whether this row holds the list's Tab stop. */
  stop?: boolean;
  hintId?: string;
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
      tabIndex={stop ? 0 : -1}
      aria-describedby={hintId}
      onClick={() => onOpenProfile(member.user_id)}
      onContextMenu={(event) => {
        event.preventDefault();
        onOpenProfile(member.user_id);
      }}
      {...ROW_ITEM}
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
  coarse,
  stop,
  hintId,
  onAdd,
  onEdit,
  onDelete,
  onMenu,
}: {
  category: Category;
  manage: boolean;
  coarse: boolean;
  stop: boolean;
  hintId: string | undefined;
  onAdd: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onMenu: (anchor: HTMLElement) => void;
}) {
  const pending = isPendingId(category.id);
  const actions = manage && !pending;
  const roving = useRovingRow(actions, stop);
  const hold = useHoldMenu(actions ? onMenu : undefined);
  return (
    <div
      className={[
        "group sidebar-category-row",
        pending ? "opacity-50" : "",
      ].join(" ")}
      onContextMenu={
        actions
          ? (event) => {
              event.preventDefault();
              onMenu(event.currentTarget);
            }
          : undefined
      }
      {...hold}
      {...roving.keys}
    >
      <h3 className="sidebar-category-name">{category.name}</h3>
      {!actions ? null : coarse ? (
        <span className={TOUCH_ACTIONS}>
          <IconButton
            label={`Aktionen für Kategorie ${category.name}`}
            onClick={(event) => onMenu(event.currentTarget)}
            small
            describedBy={hintId}
            tabIndex={roving.tabIndex(0)}
          >
            <MoreIcon size={18} />
          </IconButton>
        </span>
      ) : (
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
      )}
    </div>
  );
}

export function ChannelRow({
  channel,
  active,
  inVoice,
  live,
  manage,
  coarse,
  stop,
  hintId,
  onEdit,
  onDelete,
  onMenu,
}: {
  channel: Channel;
  active: boolean;
  inVoice: boolean;
  /** Who is live in this voice channel. */
  live: readonly string[];
  manage: boolean;
  /** A touch screen: a long press instead of the hover actions. */
  coarse: boolean;
  /** Whether this row holds the list's Tab stop. */
  stop: boolean;
  hintId: string | undefined;
  onEdit: () => void;
  onDelete: () => void;
  onMenu: (anchor: HTMLElement) => void;
}) {
  const pending = isPendingId(channel.id);
  const muted = useChannelMuted(channel.id);
  const text = channel.kind === "text";
  const badge = useBadge({ channelId: channel.id });
  const unread = text && (badge.count > 0 || badge.dot);
  // The open conversation shows its draft in the composer already.
  const draft = useHasDraft(useUserId(), channel.id) && text && !active;
  const Icon = channel.kind === "voice" ? SpeakerIcon : HashIcon;
  const body = (
    <>
      <Icon size={25} className="channel-row-icon" />
      <span className="channel-row-name">{channel.name}</span>
      {draft ? (
        <span className="channel-row-draft" title="Entwurf">
          <PencilIcon size={13} />
          <span className="sr-only">Entwurf</span>
        </span>
      ) : null}
      {text ? <UnreadBadge channelId={channel.id} showDot={false} /> : null}
      {live.length > 0 ? (
        <span className="channel-row-live" title={liveSentence(live, false)}>
          <span aria-hidden="true">Live</span>
          <span className="sr-only">{liveSentence(live, false)}</span>
        </span>
      ) : null}
    </>
  );
  const rowClass = [
    "group channel-row",
    active ? "is-active" : "",
    unread ? "is-unread" : "",
    pending ? "opacity-50" : "",
    muted && text ? "is-muted" : "",
    inVoice ? "is-in-voice" : "",
  ]
    .filter(Boolean)
    .join(" ");
  const actions = manage && !pending;
  const roving = useRovingRow(actions, stop);
  const label = text ? `#${channel.name}` : channel.name;
  // A right click or a held finger: the row's actions, or for members a
  // text channel's notifications.
  const menuAt = actions
    ? onMenu
    : text
      ? () =>
          openNotificationSettings({
            scope: "channel",
            id: channel.id,
            name: label,
            serverId: channel.server_id,
          })
      : undefined;
  const hold = useHoldMenu(menuAt);

  return (
    <div className={rowClass} {...roving.keys}>
      {pending ? (
        <span className="channel-row-link">{body}</span>
      ) : (
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: channel.server_id, channelId: channel.id }}
          aria-current={active ? "page" : undefined}
          aria-describedby={hintId}
          title={inVoice ? `${channel.name} · verbunden` : undefined}
          tabIndex={roving.tabIndex(0)}
          className="channel-row-link"
          onContextMenu={
            menuAt
              ? (event) => {
                  event.preventDefault();
                  menuAt(event.currentTarget);
                }
              : undefined
          }
          {...hold}
          {...ROW_ITEM}
        >
          {body}
        </Link>
      )}
      {!actions ? null : coarse ? (
        <span className={TOUCH_ACTIONS}>
          <IconButton
            label={`Aktionen für ${label}`}
            onClick={(event) => onMenu(event.currentTarget)}
            small
            tabIndex={roving.tabIndex(1)}
          >
            <MoreIcon size={18} />
          </IconButton>
        </span>
      ) : (
        <span className="sidebar-row-actions flex shrink-0 items-center opacity-0 transition group-focus-within:opacity-100 group-hover:opacity-100">
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
      )}
    </div>
  );
}

// On a touch screen a row's actions open with a long press, as the hint
// under the list says; the "…" button stays for screen readers and shows
// once a keyboard reaches it.
const TOUCH_ACTIONS = "sr-only focus-within:not-sr-only";

function IconButton({
  label,
  onClick,
  children,
  small,
  tabIndex,
  describedBy,
}: {
  label: string;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
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
