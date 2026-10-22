import { Link } from "@tanstack/react-router";
import { useState } from "react";
import { useUserId } from "../auth/scope.ts";
import {
  ChatIcon,
  CopyIcon,
  HashIcon,
  PencilIcon,
  SpeakerIcon,
} from "../components/Icons.tsx";
import { InviteDialog } from "../components/InviteDialog.tsx";
import { MemberPanel } from "../components/MemberPanel.tsx";
import { Button } from "../components/ui/Button.tsx";
import { useHasDraft } from "../messages/drafts.ts";
import type { Badge } from "../messages/notifyLevel.ts";
import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadges } from "../messages/useBadge.ts";
import { lastChannelsFor, useLastChannel } from "../servers/lastChannel.ts";
import { overviewRoom, roomWelcome } from "../servers/overview.ts";
import { can } from "../servers/permissions.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";
import { EMPTY_OCCUPANCY, useVoiceRoster } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import { VoiceRoomPart } from "../voice/voiceRoomPart.ts";
import "./overview.css";

/** The front door uses the same live room and session as channel pages. */
export function CommunityOverview({ server }: { server: ServerDetail }) {
  const userId = useUserId();
  const byUser = useLastChannel((s) => s.byUser);
  const voice = useVoice();
  const roster = useVoiceRoster(
    (s) => s.byServer[server.id] ?? EMPTY_OCCUPANCY,
  );
  const room = overviewRoom(
    server,
    roster,
    voice,
    lastChannelsFor(byUser, userId)[server.id],
  );
  const otherRooms = server.channels.filter(
    (channel) =>
      channel.kind === "voice" &&
      channel.id !== room?.id &&
      !channel.id.startsWith("tmp:"),
  );
  const badges = useBadges();
  // What is new comes first; otherwise the sidebar's order.
  const textChannels = server.channels
    .filter(
      (channel) => channel.kind === "text" && !channel.id.startsWith("tmp:"),
    )
    .map((channel, index) => ({
      channel,
      index,
      rank: unreadRank(badges.get(channel.id)),
    }))
    .sort((a, b) => b.rank - a.rank || a.index - b.index)
    .map(({ channel }) => channel);
  const [inviting, setInviting] = useState(false);

  return (
    <div className="community-overview-layout">
      <section
        className="community-overview"
        aria-label={`${server.name} Übersicht`}
      >
        <div className="community-overview__bar">
          <ChatIcon size={18} />
          Übersicht
        </div>
        <header className="community-overview__welcome">
          <h1>{server.name}</h1>
          <p>
            Dein Feierabend beginnt hier.{" "}
            {room
              ? roomWelcome(server.members, roster, room, userId)
              : "Schön, dass du da bist."}
          </p>
        </header>
        {server.members.length === 1 ? (
          <div className="community-overview__alone">
            <p>
              <strong>Noch allein hier</strong> – lade deine Leute ein, dann ist
              hier bald mehr los.
            </p>
            <Button
              variant="primary"
              icon={<CopyIcon size={16} />}
              onClick={() => setInviting(true)}
            >
              Link kopieren
            </Button>
            <InviteDialog
              server={server}
              open={inviting}
              onClose={() => setInviting(false)}
            />
          </div>
        ) : null}
        {room ? (
          <VoiceRoomPart.Component
            server={server}
            channelId={room.id}
            channelName={room.name}
            named
            fallback={<SkeletonRows count={3} avatars />}
          />
        ) : (
          <div className="community-overview__empty">
            <SpeakerIcon size={36} />
            <h2>Noch kein Sprachraum</h2>
            <p>
              {can(server, "manage_channels")
                ? "Erstelle über die Navigation einen Sprachkanal für eure Runde."
                : "Sobald ein Sprachkanal angelegt wurde, kannst du hier dazukommen."}
            </p>
          </div>
        )}
        {otherRooms.length > 0 || textChannels.length > 0 ? (
          <section
            className="community-overview__places"
            aria-label="Weitere Orte in deiner Community"
          >
            {otherRooms.length > 0 ? (
              <>
                <h2>Auch ein Platz für dich</h2>
                <div className="community-overview__links">
                  {otherRooms.map((channel) => {
                    const count = Object.entries(roster).filter(
                      ([id, flags]) =>
                        flags.channelId === channel.id &&
                        server.members.some((member) => member.user_id === id),
                    ).length;
                    return (
                      <Link
                        key={channel.id}
                        to="/s/$serverId/c/$channelId"
                        params={{ serverId: server.id, channelId: channel.id }}
                        className="community-overview__place"
                      >
                        <SpeakerIcon size={23} />
                        <span>
                          <strong>{channel.name}</strong>
                          <small>
                            {count === 0
                              ? "Gerade ist es ruhig"
                              : `${count} ${count === 1 ? "Person ist" : "Personen sind"} da`}
                          </small>
                        </span>
                      </Link>
                    );
                  })}
                </div>
              </>
            ) : null}
            {textChannels.length > 0 ? (
              <>
                <h2>Lieber schreiben?</h2>
                <div className="community-overview__links">
                  {textChannels.map((channel) => (
                    <TextPlace
                      key={channel.id}
                      channel={channel}
                      badge={badges.get(channel.id)}
                    />
                  ))}
                </div>
              </>
            ) : null}
          </section>
        ) : null}
      </section>
      {/* The front door shows who is around, even where the list is
          hidden next to conversations. */}
      <MemberPanel server={server} forceVisible />
    </div>
  );
}

const unreadRank = (badge: Badge | undefined) =>
  !badge ? 0 : badge.count > 0 ? 2 : badge.dot ? 1 : 0;

function TextPlace({
  channel,
  badge,
}: {
  channel: Channel;
  badge: Badge | undefined;
}) {
  const draft = useHasDraft(useUserId(), channel.id);
  return (
    <Link
      to="/s/$serverId/c/$channelId"
      params={{ serverId: channel.server_id, channelId: channel.id }}
      className={`community-overview__place${unreadRank(badge) > 0 ? " is-unread" : ""}`}
    >
      <HashIcon size={23} />
      <strong>{channel.name}</strong>
      {draft ? (
        <span className="channel-row-draft" title="Entwurf">
          <PencilIcon size={13} />
          <span className="sr-only">Entwurf</span>
        </span>
      ) : null}
      <UnreadBadge channelId={channel.id} />
    </Link>
  );
}
