// Right-hand member list. Presence lives here so a status flip never
// reflows the message pane (issue 8).

import { useMemo } from "react";

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
              {group.members.map((member) => (
                <li
                  key={member.user_id}
                  className="flex items-center gap-2 rounded-md px-2 py-1.5"
                >
                  <PresenceAvatar
                    name={member.name}
                    url={member.avatar_url}
                    status={group.group}
                  />
                  <span className="min-w-0 flex-1 truncate text-sm">
                    {member.name}
                  </span>
                </li>
              ))}
            </ul>
          </li>
        ))}
      </ul>
    </aside>
  );
}
