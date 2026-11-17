// Real presence and voice occupancy stay independent of message scroll state.
// The list is virtual (#613): only the rows in view are in the page, so a
// server with thousands of members online stays light.
import { measureElement, useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { useSession } from "../auth/session.ts";
import type { User } from "../auth/types.ts";
import { useT } from "../i18n/catalog.ts";
import { members as membersCatalog } from "../i18n/catalogs/members.ts";
import { useCurrentLocale } from "../i18n/locale.ts";
import type { Member, ServerDetail } from "../servers/types.ts";
import {
  EMPTY_LIVE,
  isLiveIn,
  useVoiceRoster,
  voiceOf,
  type LiveMap,
  type VoiceFlags,
} from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import {
  groupMembers,
  presenceOf,
  usePresenceStore,
  type PresenceGroup,
} from "../ws/live.ts";
import { LiveIcon, UsersIcon } from "./Icons.tsx";
import {
  memberListRows,
  memberRowEstimate,
  type MemberListRow,
} from "./memberListRows.ts";
import { toggleMemberPanel, useMemberPanelHidden } from "./memberPanelState.ts";
import { MemberProfileDialog } from "./MemberProfileDialog.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import {
  LIST_ROW,
  listStop,
  moveWithinList,
  neighbourRow,
  ROW_ITEM,
} from "./rowKeys.ts";
import { VoiceStateIcons } from "./VoiceStateIcons.tsx";
import { WorkspaceDrawer } from "./WorkspaceNavigation.tsx";

/** Header button: show or hide the member list on wide screens. */
export function MemberPanelToggle() {
  const t = useT(membersCatalog);
  const hidden = useMemberPanelHidden();
  return (
    <button
      type="button"
      className={`lr-header-icon member-panel-toggle${hidden ? "" : " is-on"}`}
      aria-pressed={!hidden}
      aria-label={t("toggle.label")}
      title={hidden ? t("toggle.show") : t("toggle.hide")}
      onClick={() => toggleMemberPanel()}
    >
      <UsersIcon size={18} />
    </button>
  );
}

export function MemberPanel({
  server,
  forceVisible = false,
}: {
  server: ServerDetail;
  /** Shown on wide screens even when the user hid the list next to
   * conversations. */
  forceVisible?: boolean;
}) {
  const t = useT(membersCatalog);
  const hidden = useMemberPanelHidden() && !forceVisible;
  const serverId = server.id;
  const members = server.members;
  const meUser = useSession((state) => state.user);
  const me = meUser?.id;
  const byServer = usePresenceStore((state) => state.byServer);
  const roster = useVoiceRoster((state) => state.byServer);
  const live = useVoiceRoster((state) => state.live);
  const session = useVoice(
    useShallow((state) => ({
      status: state.status,
      serverId: state.serverId,
      channelId: state.channelId,
      muted: state.muted,
      deafened: state.deafened,
    })),
  );
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [profile, setProfile] = useState<{
    serverId: string;
    memberId: string;
  } | null>(null);
  const drawerId = useId();
  const names = useMemo(
    () => new Map(server.channels.map((channel) => [channel.id, channel.name])),
    [server.channels],
  );
  // The group headings and their order are in the current language.
  const locale = useCurrentLocale();
  const groups = useMemo(
    () => groupMembers(members, (id) => presenceOf(byServer, serverId, id)),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- see locale
    [members, byServer, serverId, locale],
  );

  return (
    <>
      <button
        type="button"
        className="member-drawer-trigger"
        aria-label={t("panel.countLabel", { count: members.length })}
        aria-expanded={drawerOpen}
        aria-controls={drawerId}
        onClick={() => setDrawerOpen(true)}
      >
        <UsersIcon size={18} />
        <span className="member-drawer-trigger-label">{t("panel.title")}</span>
        <span className="member-drawer-trigger-count">{members.length}</span>
      </button>
      <WorkspaceDrawer
        id={drawerId}
        title={t("panel.title")}
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        breakpoint={1130}
        className={`member-panel-drawer${hidden ? " is-collapsed" : ""}`}
      >
        <aside className="member-panel" aria-label={t("panel.title")}>
          <header className="member-panel-heading">
            <h2>{t("panel.title")}</h2>
            <p>{members.length}</p>
          </header>
          <MemberList
            groups={groups}
            serverId={serverId}
            me={me}
            meUser={meUser}
            roster={roster}
            live={live[serverId] ?? EMPTY_LIVE}
            session={session}
            names={names}
            onOpenProfile={(memberId) => {
              setDrawerOpen(false);
              setProfile({ serverId, memberId });
            }}
          />
        </aside>
      </WorkspaceDrawer>
      <MemberProfileDialog
        server={server}
        memberId={profile?.serverId === serverId ? profile.memberId : null}
        onClose={() => setProfile(null)}
      />
    </>
  );
}

type VoiceSession = {
  status: string;
  serverId: string | null;
  channelId: string | null;
  muted: boolean;
  deafened: boolean;
};

function MemberList({
  groups,
  serverId,
  me,
  meUser,
  roster,
  live,
  session,
  names,
  onOpenProfile,
}: {
  groups: { group: PresenceGroup; label: string; members: Member[] }[];
  serverId: string;
  me: string | undefined;
  meUser: User | null;
  roster: Record<string, Record<string, VoiceFlags>>;
  live: LiveMap;
  session: VoiceSession;
  names: Map<string, string>;
  onOpenProfile: (memberId: string) => void;
}) {
  const t = useT(membersCatalog);
  const scrollRef = useRef<HTMLDivElement>(null);
  const rows = useMemo(() => memberListRows(groups), [groups]);
  // Not on the React Compiler; the warning is about memoising its return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => memberRowEstimate(rows[index]),
    getItemKey: (index) => rows[index]?.key ?? index,
    // A hidden list (the drawer closed, or still closed on the first render
    // on a wide screen) measures every row as 0. That size would stay
    // cached, and the list would jump once it shows: keep the guess.
    measureElement: (element, entry, instance) =>
      measureElement(element, entry, instance) ||
      memberRowEstimate(rows[instance.indexFromElement(element)]),
    overscan: 10,
    // The first rows render before the list is measured.
    initialRect: { width: 0, height: 600 },
  });
  const items = virtualizer.getVirtualItems();
  // The list is one Tab stop; ↑ and ↓ move between the members. The key
  // hint is read once, on the first member, not on every row.
  const hintId = useId();
  const hintKey = rows.find((row) => row.kind === "member")?.key;
  const [focusedKey, setFocusedKey] = useState<string | null>(null);
  const stop = listStop(
    items.flatMap((item) => {
      const row = rows[item.index];
      return row?.kind === "member" ? [row.key] : [];
    }),
    focusedKey,
    undefined,
  );
  // The row that held focus. When it leaves the page (the member left, or
  // their status moved them out of view) focus would fall to the page; it
  // goes to the row now in its place instead, unless the user moved on.
  const lastFocused = useRef<{ element: HTMLElement; index: number } | null>(
    null,
  );
  useEffect(() => {
    const last = lastFocused.current;
    if (!last || last.element.isConnected) return;
    lastFocused.current = null;
    const active = document.activeElement;
    if (active && active !== document.body) return;
    if (scrollRef.current) focusMemberNear(scrollRef.current, last.index);
  }, [rows]);

  return (
    <div ref={scrollRef} className="member-panel-list">
      <p id={hintId} className="sr-only">
        {t("panel.keyHint")}
      </p>
      <div
        role="list"
        aria-label={t("panel.title")}
        style={{ height: virtualizer.getTotalSize() }}
        className="relative"
        onKeyDown={moveWithinList}
        onBlur={(event) => {
          const target = event.target as HTMLElement;
          if (
            target.isConnected &&
            !event.currentTarget.contains(event.relatedTarget as Node | null)
          )
            lastFocused.current = null;
        }}
      >
        {items.map((item) => {
          const row = rows[item.index];
          if (!row) return null;
          return (
            <div
              key={item.key}
              ref={virtualizer.measureElement}
              data-index={item.index}
              role="listitem"
              aria-setsize={rows.length}
              aria-posinset={item.index + 1}
              className={memberRowClass(row)}
              style={{ transform: `translateY(${item.start}px)` }}
              {...(row.kind === "member" ? LIST_ROW : {})}
              onFocus={(event) => {
                if (row.kind !== "member") return;
                lastFocused.current = {
                  element: event.target as HTMLElement,
                  index: item.index,
                };
                setFocusedKey(row.key);
              }}
            >
              {row.kind === "heading" ? (
                <h3 className="member-group-heading">
                  {row.label} — {row.count}
                </h3>
              ) : (
                <MemberRow
                  listed={row.member}
                  group={row.group}
                  serverId={serverId}
                  me={me}
                  meUser={meUser}
                  roster={roster}
                  live={live}
                  session={session}
                  names={names}
                  tabStop={row.key === stop}
                  hintId={row.key === hintKey ? hintId : undefined}
                  onOpenProfile={onOpenProfile}
                />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** Focuses the mounted member row nearest `index`: the one now in its
 * place, else the nearest before it. */
function focusMemberNear(list: HTMLElement, index: number): void {
  const buttons = new Map<number, HTMLElement>();
  for (const row of list.querySelectorAll<HTMLElement>("[data-index]")) {
    const button = row.querySelector<HTMLElement>("[data-row-item]");
    if (button) buttons.set(Number(row.dataset.index), button);
  }
  const target = neighbourRow([...buttons.keys()], index);
  if (target !== null) buttons.get(target)?.focus();
}

function memberRowClass(row: MemberListRow<Member>): string {
  if (row.kind === "heading")
    return `member-list-row is-heading${row.first ? " is-first" : ""}`;
  return `member-list-row member-row${row.group === "x" ? " is-offline" : ""}`;
}

function MemberRow({
  listed,
  group,
  serverId,
  me,
  meUser,
  roster,
  live,
  session,
  names,
  tabStop,
  hintId,
  onOpenProfile,
}: {
  listed: Member;
  group: PresenceGroup;
  serverId: string;
  me: string | undefined;
  meUser: User | null;
  roster: Record<string, Record<string, VoiceFlags>>;
  live: LiveMap;
  session: VoiceSession;
  names: Map<string, string>;
  /** Whether this row holds the list's one Tab stop. */
  tabStop: boolean;
  /** The key hint, on the list's first member. */
  hintId: string | undefined;
  onOpenProfile: (memberId: string) => void;
}) {
  const t = useT(membersCatalog);
  const self = listed.user_id === me;
  // Your own row follows your edits without a refetch.
  const member =
    self && meUser
      ? {
          ...listed,
          name: meUser.name,
          avatar_url: meUser.avatar_url,
          status_text: meUser.status_text,
          accent_color: meUser.accent_color,
          avatar_decoration: meUser.avatar_decoration,
        }
      : listed;
  const flags = flagsFor(roster, serverId, member.user_id, me, session);
  const isLive = isLiveIn(live, member.user_id);
  return (
    <button
      type="button"
      title={t("row.profileLabel", { name: member.name })}
      aria-label={t("row.profileLabel", { name: member.name })}
      aria-describedby={hintId}
      aria-haspopup="dialog"
      tabIndex={tabStop ? 0 : -1}
      onClick={() => onOpenProfile(member.user_id)}
      className="member-profile-button"
      {...ROW_ITEM}
    >
      <PresenceAvatar
        name={member.name}
        url={member.avatar_url}
        status={group}
        size="md"
        className="member-presence-avatar"
        decoration={member.avatar_decoration}
        accent={member.accent_color}
      />
      <span className="member-description">
        <strong>
          {member.name}
          {self && <small> · {t("row.you")}</small>}
        </strong>
        {/* The group's heading says who is online; a row repeats only what
            is its own. */}
        {member.status_text ? (
          <span className="member-status-text" title={member.status_text}>
            {member.status_text}
          </span>
        ) : null}
        {isLive ? (
          <small>
            {flags
              ? t("row.liveIn", {
                  channel: names.get(flags.channelId) ?? t("row.aVoiceChannel"),
                })
              : t("row.live")}
          </small>
        ) : flags ? (
          <small>{names.get(flags.channelId) ?? t("row.inVoice")}</small>
        ) : null}
      </span>
      <span className="member-media-state">
        <VoiceStateIcons
          inVoice={flags !== null}
          muted={flags?.muted ?? false}
          deafened={flags?.deafened ?? false}
          channelName={flags ? names.get(flags.channelId) : undefined}
        />
        {isLive && (
          <LiveIcon size={20} aria-label={t("row.live")} aria-hidden={false} />
        )}
      </span>
    </button>
  );
}

function flagsFor(
  roster: Record<string, Record<string, VoiceFlags>>,
  serverId: string,
  userId: string,
  selfId: string | undefined,
  session: VoiceSession,
): VoiceFlags | null {
  if (
    selfId &&
    userId === selfId &&
    session.status === "joined" &&
    session.serverId === serverId &&
    session.channelId
  ) {
    return {
      channelId: session.channelId,
      muted: session.muted,
      deafened: session.deafened,
    };
  }
  return voiceOf(roster, serverId, userId);
}
