// Right-hand member list. Presence lives here so a status flip never
// reflows the message pane (issue 8). Click opens a 1:1 DM like a channel.
// Voice flags sit in a reserved slot so mute/deafen never shift the
// avatar or name (issue 12).

import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useMemo } from "react";

import { useSession } from "../auth/session.ts";
import { findCachedDm, prefetchDms, useOpenDm } from "../dms/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useVoiceRoster, voiceOf, type VoiceFlags } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { groupMembers, presenceOf, usePresenceStore } from "../ws/live.ts";
import { MemberActions } from "./MemberActions.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";
import { VoiceStateIcons } from "./VoiceStateIcons.tsx";

export function MemberPanel({ server }: { server: ServerDetail }) {
  const serverId = server.id;
  const members = server.members;
  const channels = server.channels;
  const me = useSession((s) => s.user?.id);
  const byServer = usePresenceStore((s) => s.byServer);
  const roster = useVoiceRoster((s) => s.byServer);
  const session = useVoice();
  const client = useQueryClient();
  const navigate = useNavigate();
  const openDm = useOpenDm();
  const names = useMemo(() => {
    const map = new Map<string, string>();
    for (const channel of channels) map.set(channel.id, channel.name);
    return map;
  }, [channels]);
  const groups = useMemo(
    () => groupMembers(members, (id) => presenceOf(byServer, serverId, id)),
    [members, byServer, serverId],
  );

  const goDm = (peerId: string) => {
    if (peerId === me) return;
    const cached = findCachedDm(client, peerId);
    if (cached) {
      void navigate({
        to: "/d/$channelId",
        params: { channelId: cached.id },
      });
      return;
    }
    openDm.mutate(peerId, {
      onSuccess: (dm) =>
        void navigate({
          to: "/d/$channelId",
          params: { channelId: dm.id },
        }),
    });
  };

  return (
    <aside
      aria-label="Mitglieder"
      className="flex h-full w-56 shrink-0 flex-col border-l border-neutral-200 dark:border-neutral-700 bg-white dark:bg-neutral-900"
    >
      <header className="flex h-12 shrink-0 items-center px-3">
        <h2 className="text-xs font-semibold tracking-wide text-neutral-500 dark:text-neutral-400 uppercase">
          Mitglieder — {members.length}
        </h2>
      </header>
      <ul className="flex-1 overflow-y-auto px-2 pb-3">
        {groups.map((group) => (
          <li key={group.group} className="mb-3">
            <p className="px-2 pb-1 text-[11px] font-semibold tracking-wide text-neutral-400 dark:text-neutral-500 uppercase">
              {group.label} — {group.members.length}
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
                return (
                  <li
                    key={member.user_id}
                    className="flex items-center gap-1 rounded-md px-2 py-1.5"
                  >
                    <button
                      type="button"
                      disabled={self}
                      title={self ? member.name : `Nachricht an ${member.name}`}
                      onMouseEnter={() => prefetchDms(client)}
                      onFocus={() => prefetchDms(client)}
                      onClick={() => goDm(member.user_id)}
                      className={[
                        "flex h-9 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left",
                        self
                          ? "cursor-default"
                          : "hover:bg-neutral-100 dark:hover:bg-neutral-800",
                      ].join(" ")}
                    >
                      <PresenceAvatar
                        name={member.name}
                        url={member.avatar_url}
                        status={group.group}
                      />
                      <span className="min-w-0 flex-1 truncate text-sm">
                        {member.name}
                      </span>
                      <VoiceStateIcons
                        inVoice={flags !== null}
                        muted={flags?.muted ?? false}
                        deafened={flags?.deafened ?? false}
                        channelName={
                          flags ? names.get(flags.channelId) : undefined
                        }
                      />
                    </button>
                    <MemberActions server={server} member={member} />
                  </li>
                );
              })}
            </ul>
          </li>
        ))}
      </ul>
    </aside>
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
