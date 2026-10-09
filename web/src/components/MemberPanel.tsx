// Real presence and voice occupancy stay independent of message scroll state.
import { useId, useMemo, useState } from "react";

import { useSession } from "../auth/session.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useVoiceRoster, voiceOf, type VoiceFlags } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { groupMembers, presenceOf, usePresenceStore } from "../ws/live.ts";
import { ScreenIcon, UsersIcon } from "./Icons.tsx";
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

export function MemberPanel({ server }: { server: ServerDetail }) {
  const hidden = useMemberPanelHidden();
  const serverId = server.id;
  const members = server.members;
  const me = useSession((state) => state.user?.id);
  const byServer = usePresenceStore((state) => state.byServer);
  const roster = useVoiceRoster((state) => state.byServer);
  const live = useVoiceRoster((state) => state.live);
  const session = useVoice();
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
        aria-expanded={drawerOpen}
        aria-controls={drawerId}
        onClick={() => setDrawerOpen(true)}
      >
        Mitglieder<span>{members.length}</span>
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
            <h2>Gerade da</h2>
            <p>
              {members.length}{" "}
              {members.length === 1 ? "Mitglied" : "Mitglieder"}
            </p>
          </header>
          <ul className="member-panel-list">
            {groups.map((group) => (
              <li key={group.group} className="member-group">
                <p className="member-group-heading">
                  {group.label} <span>{group.members.length}</span>
                </p>
                <ul>
                  {group.members.map((member) => {
                    const self = member.user_id === me;
                    const flags = flagsFor(
                      roster,
                      serverId,
                      member.user_id,
                      me,
                      session,
                    );
                    const isLive = Object.values(live[serverId] ?? {}).includes(
                      member.user_id,
                    );
                    return (
                      <li key={member.user_id} className="member-row">
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
                          />
                          <span className="member-description">
                            <strong>
                              {member.name}
                              {self && <small> · Du</small>}
                            </strong>
                            <span>{group.label}</span>
                            {isLive ? (
                              <small>Teilt live seinen Bildschirm</small>
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
                              <ScreenIcon
                                size={20}
                                aria-label="Live-Bildschirmübertragung"
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
