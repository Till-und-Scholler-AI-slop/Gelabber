// `/s/$serverId/c/$channelId`: header, virtualised messages, members.
// Voice uses the own-protocol room, not LiveKit.

import { useParams } from "@tanstack/react-router";
import { useEffect } from "react";

import { HashIcon, SpeakerIcon } from "../components/Icons.tsx";
import { MemberPanel } from "../components/MemberPanel.tsx";
import { MessagePane } from "../components/MessagePane.tsx";
import { Redirect } from "../components/Redirect.tsx";
import { TypingBar } from "../components/TypingBar.tsx";
import { VoiceRoom } from "../voice/VoiceRoom.tsx";
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
  const remember = useLastChannel((s) => s.remember);
  const channel = server?.channels.find((c) => c.id === channelId);

  useEffect(() => {
    if (channel) remember(serverId, channel.id);
  }, [channel, serverId, remember]);

  if (!server) return null;
  if (!channel) {
    return <Redirect to="/s/$serverId" params={{ serverId }} />;
  }

  const Icon = channel.kind === "voice" ? SpeakerIcon : HashIcon;

  return (
    <div className="flex min-h-0 flex-1 overflow-hidden">
      <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
        <header className="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-200 bg-white px-4">
          <Icon size={18} className="text-neutral-400" />
          <h1 className="truncate font-semibold tracking-tight">
            {channel.name}
          </h1>
        </header>
        {channel.kind === "voice" ? (
          <VoiceRoom
            server={server}
            channelId={channel.id}
            channelName={channel.name}
          />
        ) : (
          <div className="relative min-h-0 flex-1">
            <div
              data-testid="message-pane"
              className="absolute inset-0 flex min-h-0 flex-col"
            >
              <TextChat server={server} channel={channel} />
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
}: {
  server: ServerDetail;
  channel: Channel;
}) {
  const canWrite = can(server, "send_messages");
  const typing = useTypingInput(server.id, channel.id, canWrite);
  return (
    <MessagePane
      key={channel.id}
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
