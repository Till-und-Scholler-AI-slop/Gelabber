// Kick / ban of a member. Hidden unless the caller holds `manage_server`
// and the target is not the owner or themselves. Never one click away:
// both sit behind a disclosure or a menu, and both ask first.

import { useRef, useState } from "react";

import { useSession } from "../auth/session.ts";
import { can } from "../servers/permissions.ts";
import { useBanMember, useKickMember } from "../servers/queries.ts";
import type { Member, Server } from "../servers/types.ts";
import { confirmAction } from "./confirm.ts";
import { MoreIcon } from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { Popover } from "./ui/Popover.tsx";
import "./memberActions.css";

type ModerationProps = {
  server: Pick<Server, "id" | "role" | "permissions">;
  member: Member;
};

function useModeration({ server, member }: ModerationProps) {
  const me = useSession((s) => s.user?.id);
  const kickMember = useKickMember(server.id);
  const banMember = useBanMember(server.id);
  const allowed =
    can(server, "manage_server") &&
    member.role !== "owner" &&
    member.user_id !== me;
  return {
    allowed,
    // Without a session stamp the row is not dropped optimistically, so the
    // buttons stay on screen: one kick or ban at a time, never a second one.
    busy: kickMember.isPending || banMember.isPending,
    kick: async () => {
      if (
        await confirmAction({
          title: `${member.name} vom Server entfernen?`,
          message: "Mit einem neuen Link können sie wieder beitreten.",
          confirmLabel: "Entfernen",
        })
      )
        kickMember.mutate(member.user_id);
    },
    ban: async () => {
      if (
        await confirmAction({
          title: `${member.name} sperren?`,
          message:
            "Sie können nicht wieder beitreten, auch nicht mit einem Einladungslink.",
          confirmLabel: "Sperren",
        })
      )
        banMember.mutate(member.user_id);
    },
  };
}

/** In a member's profile: a collapsed "Moderation" section below a divider. */
export function MemberActions(props: ModerationProps) {
  const { allowed, busy, kick, ban } = useModeration(props);
  if (!allowed) return null;
  return (
    <details className="member-moderation">
      <summary>Moderation</summary>
      <div className="member-moderation__actions">
        <Button
          variant="danger-outline"
          size="sm"
          onClick={kick}
          disabled={busy}
        >
          Vom Server entfernen
        </Button>
        <Button variant="danger" size="sm" onClick={ban} disabled={busy}>
          Sperren
        </Button>
      </div>
    </details>
  );
}

/** On a member row: a "…" button whose menu holds kick and ban. */
export function MemberActionsMenu(props: ModerationProps) {
  const { allowed, busy, kick, ban } = useModeration(props);
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  if (!allowed) return null;
  const label = `Aktionen für ${props.member.name}`;
  const choose = (action: () => Promise<void>) => () => {
    setOpen(false);
    void action();
  };
  return (
    <>
      <IconButton
        ref={button}
        label={label}
        icon={<MoreIcon size={18} />}
        size="sm"
        aria-haspopup="dialog"
        aria-expanded={open}
        disabled={busy}
        onClick={() => setOpen((value) => !value)}
      />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={button}
        label={label}
        placement="bottom-end"
      >
        <div className="member-menu">
          <button
            type="button"
            className="member-menu__item"
            onClick={choose(kick)}
          >
            Vom Server entfernen
          </button>
          <button
            type="button"
            className="member-menu__item member-menu__item--danger"
            onClick={choose(ban)}
          >
            Sperren
          </button>
        </div>
      </Popover>
    </>
  );
}
