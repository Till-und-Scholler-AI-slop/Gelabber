// `/s/$serverId/c/$channelId`: header, virtualised messages, members.
// Voice uses the own-protocol room, not LiveKit.

import { useParams } from "@tanstack/react-router";
import { useEffect, type ReactNode } from "react";

import { useUserId } from "../auth/scope.ts";
import { HashIcon, SpeakerIcon } from "../components/Icons.tsx";
import { MemberPanel, MemberPanelToggle } from "../components/MemberPanel.tsx";
import { MessagePane } from "../components/MessagePane.tsx";
import "../components/chat.css";
import { Redirect } from "../components/Redirect.tsx";
import { TypingBar } from "../components/TypingBar.tsx";
import { VoiceRoom } from "../voice/VoiceRoom.tsx";
import { LiveHint } from "../voice/LiveHint.tsx";
import { useLastChannel } from "../servers/lastChannel.ts";
import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";
import { useTypingInput } from "../ws/useLive.ts";

export function ChannelPage() {
  const { serverId, channelId } = useParams({
    from: "/workspace/s/$serverId/c/$channelId",
  });
  const { data: server } = useServer(serverId);
  const userId = useUserId();
  const remember = useLastChannel((s) => s.remember);
  const channel = server?.channels.find((c) => c.id === channelId);

  useEffect(() => {
    if (channel && userId) remember(userId, serverId, channel.id);
  }, [channel, serverId, remember, userId]);

  if (!server) return null;
  if (!channel) {
    return <Redirect to="/s/$serverId" params={{ serverId }} />;
  }

  const Icon = channel.kind === "voice" ? SpeakerIcon : HashIcon;

  const title = (
    <>
      <Icon size={18} />
      <h1>{channel.name}</h1>
      <LiveHint server={server} variant="pill" />
    </>
  );

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <div className="lr-channel-content flex min-w-0 flex-1 flex-col overflow-hidden">
        {channel.kind === "voice" ? (
          <>
            <header className="lr-channel-header">
              {title}
              <span className="lr-channel-header-spacer" />
              <MemberPanelToggle />
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto">
              <VoiceRoom
                server={server}
                channelId={channel.id}
                channelName={channel.name}
              />
            </div>
          </>
        ) : (
          <div className="relative min-h-0 flex-1">
            <div
              data-testid="message-pane"
              className="absolute inset-0 flex min-h-0 flex-col"
            >
              <TextChat
                server={server}
                channel={channel}
                title={title}
                notice={<LiveHint server={server} />}
                actions={<MemberPanelToggle />}
              />
            </div>
          </div>
        )}
      </div>
      <MemberPanel server={server} />
    </div>
  );
}

function TextChat({
  server,
  channel,
  title,
  notice,
  actions,
}: {
  server: ServerDetail;
  channel: Channel;
  title: ReactNode;
  notice: ReactNode;
  actions: ReactNode;
}) {
  const canWrite = can(server, "send_messages");
  const typing = useTypingInput(server.id, channel.id, canWrite);
  return (
    <MessagePane
      key={channel.id}
      title={title}
      notice={notice}
      actions={actions}
      channelId={channel.id}
      channelName={channel.name}
      canSend={canWrite}
      canSendFiles={can(server, "send_files")}
      canModerate={can(server, "manage_messages")}
      footer={<TypingBar channelId={channel.id} members={server.members} />}
      onDraftChange={typing.onChange}
      onDraftStop={typing.stop}
    />
  );
}
