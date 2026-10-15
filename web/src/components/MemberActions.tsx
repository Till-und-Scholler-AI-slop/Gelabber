// Kick / ban on a member row. Hidden unless the caller holds
// `manage_server` and the target is not the owner or themselves.

import { useSession } from "../auth/session.ts";
import { can } from "../servers/permissions.ts";
import { useBanMember, useKickMember } from "../servers/queries.ts";
import type { Member, Server } from "../servers/types.ts";

// Big enough to hit, and a finger's width on touch screens.
const ACTION =
  "inline-flex min-h-8 items-center rounded-lg border px-3 text-sm font-medium transition pointer-coarse:min-h-11 pointer-coarse:px-4 disabled:cursor-default disabled:opacity-50";

export function MemberActions({
  server,
  member,
  className = "",
}: {
  server: Pick<Server, "id" | "role" | "permissions">;
  member: Member;
  /** Extra classes for the row of buttons, e.g. its place in a wrapping row. */
  className?: string;
}) {
  const me = useSession((s) => s.user?.id);
  const kick = useKickMember(server.id);
  const ban = useBanMember(server.id);
  if (!can(server, "manage_server")) return null;
  if (member.role === "owner" || member.user_id === me) return null;
  // Without a session stamp the row is not dropped optimistically, so the
  // buttons stay on screen: one kick or ban at a time, never a second one.
  const busy = kick.isPending || ban.isPending;

  return (
    <span className={`flex shrink-0 items-center gap-2 ${className}`}>
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
        disabled={busy}
        aria-label={`Kick ${member.name}`}
        className={`${ACTION} border-neutral-300 dark:border-neutral-700 text-neutral-700 dark:text-neutral-300 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100`}
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
        disabled={busy}
        aria-label={`Ban ${member.name}`}
        className={`${ACTION} border-red-300 dark:border-red-800 text-red-700 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-950`}
      >
        Ban
      </button>
    </span>
  );
}
