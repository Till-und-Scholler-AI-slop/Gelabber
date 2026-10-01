import { Link } from "@tanstack/react-router";
import { useUserId } from "../auth/scope.ts";
import { HashIcon, SpeakerIcon } from "../components/Icons.tsx";
import { MemberPanel } from "../components/MemberPanel.tsx";
import { lastChannelsFor, useLastChannel } from "../servers/lastChannel.ts";
import { overviewRoom, roomWelcome } from "../servers/overview.ts";
import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { EMPTY_OCCUPANCY, useVoiceRoster } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { VoiceRoom } from "../voice/VoiceRoom.tsx";
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
  const textChannels = server.channels.filter(
    (channel) => channel.kind === "text" && !channel.id.startsWith("tmp:"),
  );

  return (
    <div className="community-overview-layout">
      <section
        className="community-overview"
        aria-label={`${server.name} Übersicht`}
      >
        <div className="community-overview__bar">{server.name}</div>
        <header className="community-overview__welcome">
          <h1>Dein Feierabend beginnt hier.</h1>
          <p>
            {room
              ? roomWelcome(server.members, roster, room, userId)
              : "Ein Platz für deine Leute. Schön, dass du da bist."}
          </p>
        </header>
        {room ? (
          <VoiceRoom
            server={server}
            channelId={room.id}
            channelName={room.name}
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
                    <Link
                      key={channel.id}
                      to="/s/$serverId/c/$channelId"
                      params={{ serverId: server.id, channelId: channel.id }}
                      className="community-overview__place"
                    >
                      <HashIcon size={23} />
                      <strong>{channel.name}</strong>
                    </Link>
                  ))}
                </div>
              </>
            ) : null}
          </section>
        ) : null}
      </section>
      <MemberPanel server={server} />
    </div>
  );
}
