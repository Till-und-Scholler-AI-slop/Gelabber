// `/s/$serverId/c/$channelId`: header with the channel, body reserved for
// the chat/voice tickets. Remembers the channel for the next server switch.

import { useParams } from "@tanstack/react-router";

import { Redirect } from "../components/Redirect.tsx";
import { useEffect } from "react";

import { HashIcon, SpeakerIcon } from "../components/Icons.tsx";
import { useLastChannel } from "../servers/lastChannel.ts";
import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";

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
    // Deleted (here or elsewhere): fall back to the server's default.
    return <Redirect to="/s/$serverId" params={{ serverId }} />;
  }

  const Icon = channel.kind === "voice" ? SpeakerIcon : HashIcon;
  const hint =
    channel.kind === "voice"
      ? can(server, "join_voice")
        ? "Voice und Go Live kommen mit den nächsten Tickets — eigener SFU, kein LiveKit."
        : "Du hast in diesem Server kein Recht, Voice beizutreten."
      : can(server, "send_messages")
        ? "Nachrichten kommen mit dem Chat-Ticket."
        : "Du hast in diesem Server kein Schreibrecht.";

  return (
    <>
      <header className="flex h-12 shrink-0 items-center gap-2 border-b border-neutral-200 bg-white px-4">
        <Icon size={18} className="text-neutral-400" />
        <h1 className="truncate font-semibold tracking-tight">
          {channel.name}
        </h1>
      </header>
      <div className="flex flex-1 items-center justify-center p-8 text-center">
        <div className="max-w-sm text-neutral-500">
          <p className="text-lg font-medium text-neutral-800">
            {channel.kind === "voice" ? channel.name : `#${channel.name}`}
          </p>
          <p className="mt-2 text-sm">{hint}</p>
        </div>
      </div>
    </>
  );
}
