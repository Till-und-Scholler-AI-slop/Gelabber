import { useQueryClient } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef } from "react";
import { stampHolds, takeStamp } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { findCachedDm, useOpenDm } from "../dms/queries.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useVoiceRoster, voiceOf } from "../voice/roster.ts";
import { presenceOf, usePresenceStore } from "../ws/live.ts";
import { ChatIcon, SpeakerIcon } from "./Icons.tsx";
import { MemberActions } from "./MemberActions.tsx";
import { GhostButton, Modal } from "./Modal.tsx";
import { PresenceAvatar } from "./PresenceAvatar.tsx";

export function MemberProfileDialog({
  server,
  memberId,
  onClose,
}: {
  server: ServerDetail;
  memberId: string | null;
  onClose: () => void;
}) {
  const me = useSession((s) => s.user?.id);
  const member = server.members.find((person) => person.user_id === memberId);
  const presence = usePresenceStore((s) =>
    presenceOf(s.byServer, server.id, memberId ?? ""),
  );
  const room = useVoiceRoster((s) =>
    voiceOf(s.byServer, server.id, memberId ?? ""),
  );
  const client = useQueryClient();
  const navigate = useNavigate();
  const openDm = useOpenDm();
  const request = useRef(0);
  useEffect(
    () => () => {
      request.current += 1;
    },
    [memberId],
  );
  const close = () => {
    request.current += 1;
    onClose();
  };
  const message = () => {
    if (!member || !me || member.user_id === me) return;
    const stamp = takeStamp();
    const current = ++request.current;
    const go = (id: string) => {
      if (!stampHolds(stamp) || current !== request.current) return;
      close();
      void navigate({ to: "/d/$channelId", params: { channelId: id } });
    };
    const cached = findCachedDm(client, me, member.user_id);
    if (cached) go(cached.id);
    else openDm.mutate(member.user_id, { onSuccess: (dm) => go(dm.id) });
  };
  const channel = room
    ? server.channels.find((c) => c.id === room.channelId)
    : undefined;
  const joinedAt = member ? new Date(member.joined_at) : null;
  const self = member?.user_id === me;

  return (
    <Modal
      open={Boolean(member)}
      title={self ? "Dein Profil" : `Profil von ${member?.name ?? "Mitglied"}`}
      onClose={close}
    >
      {member ? (
        <div className="member-profile">
          <div className="member-profile__person">
            <PresenceAvatar
              name={member.name}
              url={member.avatar_url}
              status={presence}
              size="lg"
            />
            <div>
              <h3>{member.name}</h3>
              <p>
                {presence === "o"
                  ? "Online"
                  : presence === "i"
                    ? "Abwesend"
                    : "Offline"}
              </p>
            </div>
          </div>
          <dl className="member-profile__details">
            <div>
              <dt>Community</dt>
              <dd>{server.name}</dd>
            </div>
            <div>
              <dt>Mitgliedschaft</dt>
              <dd>{member.role === "owner" ? "Inhaber" : "Mitglied"}</dd>
            </div>
            {joinedAt && !Number.isNaN(joinedAt.getTime()) ? (
              <div>
                <dt>Dabei seit</dt>
                <dd>
                  {new Intl.DateTimeFormat("de-DE", {
                    dateStyle: "medium",
                  }).format(joinedAt)}
                </dd>
              </div>
            ) : null}
          </dl>
          {channel ? (
            <Link
              className="member-profile__room"
              to="/s/$serverId/c/$channelId"
              params={{ serverId: server.id, channelId: channel.id }}
              onClick={close}
            >
              <SpeakerIcon size={20} />
              In {channel.name}
            </Link>
          ) : null}
          <div className="member-profile__actions">
            {self ? (
              <Link className="lr-primary" to="/profile" onClick={close}>
                Profil bearbeiten
              </Link>
            ) : (
              <button
                type="button"
                className="lr-primary"
                onClick={message}
                disabled={openDm.isPending}
              >
                <ChatIcon size={19} />
                {openDm.isPending
                  ? "Unterhaltung wird geöffnet…"
                  : "Nachricht senden"}
              </button>
            )}
            <GhostButton onClick={close}>Schließen</GhostButton>
          </div>
          <MemberActions server={server} member={member} />
        </div>
      ) : null}
    </Modal>
  );
}
