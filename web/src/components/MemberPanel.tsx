// Real presence and voice occupancy stay independent of message scroll state.
import { useId, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";

import { useSession } from "../auth/session.ts";
import type { ServerDetail } from "../servers/types.ts";
import {
  EMPTY_LIVE,
  isLiveIn,
  useVoiceRoster,
  voiceOf,
  type VoiceFlags,
} from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { groupMembers, presenceOf, usePresenceStore } from "../ws/live.ts";
import { LiveIcon, UsersIcon } from "./Icons.tsx";
import { toggleMemberPanel, useMemberPanelHidden } from "./memberPanelState.ts";
import { MemberProfileDialog } from "./MemberProfileDialog.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { VoiceStateIcons } from "./VoiceStateIcons.tsx";
import { WorkspaceDrawer } from "./WorkspaceNavigation.tsx";

/** Header button: show or hide the member list on wide screens. */
export function MemberPanelToggle() {
  const hidden = useMemberPanelHidden();
  return (
    <button
      type="button"
      className={`lr-header-icon member-panel-toggle${hidden ? "" : " is-on"}`}
      aria-pressed={!hidden}
      aria-label="Mitgliederliste"
      title={
        hidden ? "Mitgliederliste einblenden" : "Mitgliederliste ausblenden"
      }
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
  const groups = useMemo(
    () => groupMembers(members, (id) => presenceOf(byServer, serverId, id)),
    [members, byServer, serverId],
  );

  return (
    <>
      <button
        type="button"
        className="member-drawer-trigger"
        aria-label={`Mitglieder (${members.length})`}
        aria-expanded={drawerOpen}
        aria-controls={drawerId}
        onClick={() => setDrawerOpen(true)}
      >
        <UsersIcon size={18} />
        <span className="member-drawer-trigger-label">Mitglieder</span>
        <span className="member-drawer-trigger-count">{members.length}</span>
      </button>
      <WorkspaceDrawer
        id={drawerId}
        title="Mitglieder"
        open={drawerOpen}
        onClose={() => setDrawerOpen(false)}
        breakpoint={1130}
        className={`member-panel-drawer${hidden ? " is-collapsed" : ""}`}
      >
        <aside className="member-panel" aria-label="Mitglieder">
          <header className="member-panel-heading">
            <h2>Mitglieder</h2>
            <p>{members.length}</p>
          </header>
          <ul className="member-panel-list">
            {groups.map((group) => (
              <li key={group.group} className="member-group">
                <h3 className="member-group-heading">
                  {group.label} — {group.members.length}
                </h3>
                <ul>
                  {group.members.map((listed) => {
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
                    const flags = flagsFor(
                      roster,
                      serverId,
                      member.user_id,
                      me,
                      session,
                    );
                    const isLive = isLiveIn(
                      live[serverId] ?? EMPTY_LIVE,
                      member.user_id,
                    );
                    return (
                      <li
                        key={member.user_id}
                        className={`member-row${group.group === "x" ? " is-offline" : ""}`}
                      >
                        <button
                          type="button"
                          title={`Profil von ${member.name}`}
                          aria-label={`Profil von ${member.name}`}
                          aria-haspopup="dialog"
                          onClick={() => {
                            setDrawerOpen(false);
                            setProfile({ serverId, memberId: member.user_id });
                          }}
                          className="member-profile-button"
                        >
                          <PresenceAvatar
                            name={member.name}
                            url={member.avatar_url}
                            status={group.group}
                            size="md"
                            className="member-presence-avatar"
                            decoration={member.avatar_decoration}
                            accent={member.accent_color}
                          />
                          <span className="member-description">
                            <strong>
                              {member.name}
                              {self && <small> · Du</small>}
                            </strong>
                            {member.status_text ? (
                              <span
                                className="member-status-text"
                                title={member.status_text}
                              >
                                {/* The status text takes the line; screen
                                    readers still hear the presence. */}
                                <span className="sr-only">{group.label}: </span>
                                {member.status_text}
                              </span>
                            ) : (
                              <span>{group.label}</span>
                            )}
                            {isLive ? (
                              <small>
                                Live
                                {flags
                                  ? ` in ${names.get(flags.channelId) ?? "einem Sprachkanal"}`
                                  : ""}
                              </small>
                            ) : flags ? (
                              <small>
                                {names.get(flags.channelId) ?? "Im Sprachraum"}
                              </small>
                            ) : null}
                          </span>
                          <span className="member-media-state">
                            <VoiceStateIcons
                              inVoice={flags !== null}
                              muted={flags?.muted ?? false}
                              deafened={flags?.deafened ?? false}
                              channelName={
                                flags ? names.get(flags.channelId) : undefined
                              }
                            />
                            {isLive && (
                              <LiveIcon
                                size={20}
                                aria-label="Live"
                                aria-hidden={false}
                              />
                            )}
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </li>
            ))}
          </ul>
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

function flagsFor(
  roster: Record<string, Record<string, VoiceFlags>>,
  serverId: string,
  userId: string,
  selfId: string | undefined,
  session: {
    status: string;
    serverId: string | null;
    channelId: string | null;
    muted: boolean;
    deafened: boolean;
  },
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
