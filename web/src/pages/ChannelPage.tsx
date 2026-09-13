// `/s/$serverId/c/$channelId`: header, isolated message pane, reserved
// typing strip, composer (local — REST messages are issue 5), members.

import { useParams } from "@tanstack/react-router";
import { useEffect, useState, type FormEvent } from "react";

import { Redirect } from "../components/Redirect.tsx";
import { HashIcon, SpeakerIcon } from "../components/Icons.tsx";
import { MemberPanel } from "../components/MemberPanel.tsx";
import { TypingBar } from "../components/TypingBar.tsx";
import { VoiceRoom } from "../voice/VoiceRoom.tsx";
import { useLastChannel } from "../servers/lastChannel.ts";
import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
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
  const canWrite = channel.kind === "text" && can(server, "send_messages");
  const hint = can(server, "send_messages")
    ? "Nachrichten kommen mit dem Chat-Ticket."
    : "Du hast in diesem Server kein Schreibrecht.";

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
          <>
            <div className="relative min-h-0 flex-1">
              <div
                data-testid="message-pane"
                className="absolute inset-0 overflow-y-auto"
              >
                <div className="flex h-full items-center justify-center p-8 text-center">
                  <div className="max-w-sm text-neutral-500">
                    <p className="text-lg font-medium text-neutral-800">
                      #{channel.name}
                    </p>
                    <p className="mt-2 text-sm">{hint}</p>
                  </div>
                </div>
              </div>
            </div>
            <TypingBar channelId={channel.id} members={server.members} />
            <Composer
              serverId={serverId}
              channelId={channel.id}
              enabled={canWrite}
            />
          </>
        )}
      </div>
      <MemberPanel serverId={serverId} members={server.members} />
    </div>
  );
}

function Composer({
  serverId,
  channelId,
  enabled,
}: {
  serverId: string;
  channelId: string;
  enabled: boolean;
}) {
  const [value, setValue] = useState("");
  const typing = useTypingInput(serverId, channelId, enabled);

  const onSubmit = (event: FormEvent) => {
    event.preventDefault();
    setValue("");
    typing.stop();
  };

  return (
    <form
      onSubmit={onSubmit}
      className="shrink-0 border-t border-neutral-200 bg-white p-3"
    >
      <input
        value={value}
        disabled={!enabled}
        onChange={(event) => {
          const next = event.target.value;
          setValue(next);
          typing.onChange(next);
        }}
        onBlur={() => typing.stop()}
        placeholder={
          enabled ? "Nachricht schreiben…" : "Kein Schreibrecht in diesem Server."
        }
        aria-label="Nachricht"
        className="w-full rounded-lg border border-neutral-200 bg-neutral-50 px-3 py-2 text-sm outline-none focus:border-neutral-400 disabled:text-neutral-400"
      />
    </form>
  );
}
