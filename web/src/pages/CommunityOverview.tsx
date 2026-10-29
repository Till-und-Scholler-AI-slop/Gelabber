import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useId, useState } from "react";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import {
  ChatIcon,
  HashIcon,
  PencilIcon,
  SpeakerIcon,
} from "../components/Icons.tsx";
import { InviteDialog } from "../components/InviteDialog.tsx";
import { MemberPanel } from "../components/MemberPanel.tsx";
import { formatFull } from "../components/messageTime.ts";
import { Button } from "../components/ui/Button.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import { listMessages } from "../messages/api.ts";
import { useHasDraft } from "../messages/drafts.ts";
import type { MentionMember } from "../messages/mentions.ts";
import { useReadState } from "../messages/readState.ts";
import { UnreadBadge } from "../messages/UnreadBadge.tsx";
import { useBadges } from "../messages/useBadge.ts";
import { lastChannelsFor, useLastChannel } from "../servers/lastChannel.ts";
import {
  messagePreview,
  overviewLine,
  overviewRoom,
  shortWhen,
  unreadChannels,
} from "../servers/overview.ts";
import { can } from "../servers/permissions.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";
import { EMPTY_OCCUPANCY, useVoiceRoster } from "../voice/roster.ts";
import { useVoice } from "../voice/session.ts";
import { VoiceRoomPart } from "../voice/voiceRoomPart.ts";
import "./overview.css";

/** Unread channels the overview lists; the sidebar marks every one. */
const UNREAD_SHOWN = 5;

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
  const [inviting, setInviting] = useState(false);

  return (
    <div className="community-overview-layout">
      <section className="community-overview" aria-label="Übersicht">
        <div className="community-overview__bar">
          <ChatIcon size={18} />
          Übersicht
        </div>
        <div className="community-overview__body">
          <header className="community-overview__head">
            <h1>{server.name}</h1>
            <p>{overviewLine(server, roster, voice, userId)}</p>
            {server.members.length === 1 ? (
              <div className="community-overview__alone">
                <p>Noch keine weiteren Mitglieder.</p>
                <Button size="sm" onClick={() => setInviting(true)}>
                  Einladen
                </Button>
                <InviteDialog
                  server={server}
                  open={inviting}
                  onClose={() => setInviting(false)}
                />
              </div>
            ) : null}
          </header>
          {room ? (
            <VoiceRoomPart.Component
              server={server}
              channelId={room.id}
              channelName={room.name}
              overview
              fallback={<SkeletonRows count={3} avatars />}
            />
          ) : (
            <div className="community-overview__no-room">
              <h2>
                <SpeakerIcon size={18} />
                Kein Sprachkanal
              </h2>
              {can(server, "manage_channels") ? (
                <p>Erstelle einen Sprachkanal in der Seitenleiste.</p>
              ) : null}
            </div>
          )}
          <UnreadChannels server={server} />
        </div>
      </section>
      {/* The front door shows who is around, even where the list is
          hidden next to conversations. */}
      <MemberPanel server={server} forceVisible />
    </div>
  );
}

/** What is new in the text channels, newest first: one row per channel
 * with its latest message. Read and empty channels are the sidebar's. */
function UnreadChannels({ server }: { server: ServerDetail }) {
  const badges = useBadges();
  const { data } = useReadState();
  const heading = useId();
  const latest = new Map(
    (data ?? []).map((row) => [row.channel_id, row.latest_order]),
  );
  const channels = unreadChannels(server.channels, badges, latest).slice(
    0,
    UNREAD_SHOWN,
  );
  if (channels.length === 0) return null;
  const members = server.members.map((member) => ({
    id: member.user_id,
    name: member.name,
  }));
  return (
    <section className="overview-unread" aria-labelledby={heading}>
      <h2 id={heading} className="overview-unread__title">
        Ungelesen
      </h2>
      <ul className="overview-unread__list">
        {channels.map((channel) => (
          <li key={channel.id}>
            <UnreadRow
              channel={channel}
              latestOrder={latest.get(channel.id) ?? null}
              members={members}
            />
          </li>
        ))}
      </ul>
    </section>
  );
}

function UnreadRow({
  channel,
  latestOrder,
  members,
}: {
  channel: Channel;
  /** The newest message's order from the unread state: a new message
   * changes it, and the preview asks again. */
  latestOrder: number | null;
  members: readonly MentionMember[];
}) {
  const userId = useUserId();
  const generation = scopeGeneration();
  const draft = useHasDraft(userId, channel.id);
  // One message, the newest: the list endpoint's smallest page.
  const { data: message } = useQuery({
    queryKey: [
      "user",
      userId,
      generation,
      "latest-message",
      channel.id,
      latestOrder,
    ],
    queryFn: async ({ signal }) =>
      (await listMessages(channel.id, { limit: 1 }, signal)).messages.at(-1) ??
      null,
    enabled: Boolean(userId),
    staleTime: 30_000,
    retry: false,
    // The old line stays until the new one is there.
    placeholderData: keepPreviousData,
  });
  const preview = message ? messagePreview(message, members, userId) : null;
  return (
    <Link
      to="/s/$serverId/c/$channelId"
      params={{ serverId: channel.server_id, channelId: channel.id }}
      className="overview-unread__row"
    >
      <HashIcon size={18} />
      <span className="overview-unread__name">
        <span>{channel.name}</span>
        {draft ? (
          <span className="channel-row-draft" title="Entwurf">
            <PencilIcon size={13} />
            <span className="sr-only">Entwurf</span>
          </span>
        ) : null}
      </span>
      <span className="overview-unread__preview">
        {preview?.author ? (
          <span className="overview-unread__author">{preview.author}: </span>
        ) : null}
        {preview?.text}
      </span>
      {message ? (
        <time
          className="overview-unread__time"
          dateTime={message.created_at}
          title={formatFull(message.created_at)}
        >
          {shortWhen(message.created_at)}
        </time>
      ) : (
        <span className="overview-unread__time" />
      )}
      <span className="overview-unread__badge">
        <UnreadBadge channelId={channel.id} />
      </span>
    </Link>
  );
}
