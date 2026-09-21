// Kick / ban on a member row. Hidden unless the caller holds
// `manage_server` and the target is not the owner or themselves.

import { useSession } from "../auth/session.ts";
import { can } from "../servers/permissions.ts";
import { useBanMember, useKickMember } from "../servers/queries.ts";
import type { Member, Server } from "../servers/types.ts";

export function MemberActions({
  server,
  member,
}: {
  server: Pick<Server, "id" | "role" | "permissions">;
  member: Member;
}) {
  const me = useSession((s) => s.user?.id);
  const kick = useKickMember(server.id);
  const ban = useBanMember(server.id);
  if (!can(server, "manage_server")) return null;
  if (member.role === "owner" || member.user_id === me) return null;

  return (
    <span className="flex shrink-0 items-center gap-1">
      <button
        type="button"
        onClick={() => {
          if (
            window.confirm(
              `${member.name} vom Server entfernen? Mit einem neuen Link können sie wieder beitreten.`,
            )
          ) {
            kick.mutate(member.user_id);
          }
        }}
        className="rounded-md px-1.5 py-0.5 text-xs font-medium text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100"
      >
        Kick
      </button>
      <button
        type="button"
        onClick={() => {
          if (
            window.confirm(
              `${member.name} sperren? Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink.`,
            )
          ) {
            ban.mutate(member.user_id);
          }
        }}
        className="rounded-md px-1.5 py-0.5 text-xs font-medium text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-950"
      >
        Ban
      </button>
    </span>
  );
}
