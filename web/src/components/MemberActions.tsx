// Kick / ban of a member. Hidden unless the caller holds `moderate_members`
// and the target is not the owner or themselves. Never one click away:
// both sit behind a disclosure or a menu, and both ask first.

import { useRef, useState } from "react";

import { useSession } from "../auth/session.ts";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { members } from "../i18n/catalogs/members.ts";
import { can } from "../servers/permissions.ts";
import { useBanMember, useKickMember } from "../servers/queries.ts";
import type { Member, Server } from "../servers/types.ts";
import { confirmAction } from "./confirm.ts";
import { ChevronIcon, MoreIcon } from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { Popover } from "./ui/Popover.tsx";
import "./memberActions.css";

type ModerationProps = {
  server: Pick<Server, "id" | "role" | "permissions">;
  member: Member;
};

function useModeration({ server, member }: ModerationProps) {
  const t = useT(members);
  const tc = useT(common);
  const me = useSession((s) => s.user?.id);
  const kickMember = useKickMember(server.id);
  const banMember = useBanMember(server.id);
  const allowed =
    can(server, "moderate_members") &&
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
          title: t("moderation.kickTitle", { name: member.name }),
          message: t("moderation.kickMessage"),
          confirmLabel: tc("remove"),
        })
      )
        kickMember.mutate(member.user_id);
    },
    ban: async () => {
      if (
        await confirmAction({
          title: t("moderation.banTitle", { name: member.name }),
          message: t("moderation.banMessage"),
          confirmLabel: t("moderation.ban"),
        })
      )
        banMember.mutate(member.user_id);
    },
  };
}

/** In a member's profile: a collapsed "Moderation" section below a divider. */
export function MemberActions(props: ModerationProps) {
  const t = useT(members);
  const { allowed, busy, kick, ban } = useModeration(props);
  if (!allowed) return null;
  return (
    <details className="member-moderation">
      <summary>
        {t("moderation.title")}
        <ChevronIcon size={14} />
      </summary>
      <div className="member-moderation__actions">
        <Button
          variant="danger-outline"
          size="sm"
          onClick={kick}
          disabled={busy}
        >
          {t("moderation.kick")}
        </Button>
        <Button variant="danger" size="sm" onClick={ban} disabled={busy}>
          {t("moderation.ban")}
        </Button>
      </div>
    </details>
  );
}

/** On a member row: a "…" button whose menu holds kick and ban. */
export function MemberActionsMenu(props: ModerationProps) {
  const t = useT(members);
  const { allowed, busy, kick, ban } = useModeration(props);
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  if (!allowed) return null;
  const label = t("moderation.menuLabel", { name: props.member.name });
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
            {t("moderation.kick")}
          </button>
          <button
            type="button"
            className="member-menu__item member-menu__item--danger"
            onClick={choose(ban)}
          >
            {t("moderation.ban")}
          </button>
        </div>
      </Popover>
    </>
  );
}
