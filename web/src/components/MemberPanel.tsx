// Right-hand member list. Presence lives here so a status flip never
// reflows the message pane (issue 8). Click opens a 1:1 DM like a channel.

import { useNavigate } from "@tanstack/react-router";
import { useMemo } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { useSession } from "../auth/session.ts";
import {
  findCachedDm,
  prefetchDms,
  useOpenDm,
} from "../dms/queries.ts";
import type { Member } from "../servers/types.ts";
import {
  groupMembers,
  presenceOf,
  usePresenceStore,
} from "../ws/live.ts";
import { PresenceAvatar } from "./PresenceAvatar.tsx";

export function MemberPanel({
  serverId,
  members,
}: {
  serverId: string;
  members: Member[];
}) {
  const byServer = usePresenceStore((s) => s.byServer);
  const groups = useMemo(
    () => groupMembers(members, (id) => presenceOf(byServer, serverId, id)),
    [members, byServer, serverId],
  );
  const me = useSession((s) => s.user?.id);
  const client = useQueryClient();
  const navigate = useNavigate();
  const openDm = useOpenDm();

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
      className="flex h-full w-56 shrink-0 flex-col border-l border-neutral-200 bg-white"
    >
      <header className="flex h-12 shrink-0 items-center px-3">
        <h2 className="text-xs font-semibold tracking-wide text-neutral-500 uppercase">
          Mitglieder — {members.length}
        </h2>
      </header>
      <ul className="flex-1 overflow-y-auto px-2 pb-3">
        {groups.map((group) => (
          <li key={group.group} className="mb-3">
            <p className="px-2 pb-1 text-[11px] font-semibold tracking-wide text-neutral-400 uppercase">
              {group.label} — {group.members.length}
            </p>
            <ul>
              {group.members.map((member) => {
                const self = member.user_id === me;
                return (
                  <li key={member.user_id}>
                    <button
                      type="button"
                      disabled={self}
                      title={
                        self
                          ? member.name
                          : `Nachricht an ${member.name}`
                      }
                      onMouseEnter={() => prefetchDms(client)}
                      onFocus={() => prefetchDms(client)}
                      onClick={() => goDm(member.user_id)}
                      className={[
                        "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left",
                        self
                          ? "cursor-default"
                          : "hover:bg-neutral-100",
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
                    </button>
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
