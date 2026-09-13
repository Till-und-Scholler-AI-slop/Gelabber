// Voice channel body: join is a local state flip, ICE runs afterwards.

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { VoiceControls } from "../components/VoiceControls.tsx";
import { VoiceStateIcons } from "../components/VoiceStateIcons.tsx";
import { joinVoice, useVoice } from "./session.ts";
import { useVoiceRoster } from "./roster.ts";

export function VoiceRoom({
  server,
  channelId,
  channelName,
}: {
  server: ServerDetail;
  channelId: string;
  channelName: string;
}) {
  const allowed = can(server, "join_voice");
  const voice = useVoice();
  const roster = useVoiceRoster((s) => s.byServer[server.id] ?? {});
  const here = voice.status === "joined" && voice.channelId === channelId;
  const members = new Map(
    server.members.map((member) => [member.user_id, member]),
  );
  const occupants = Object.entries(roster).filter(
    ([, flags]) => flags.channelId === channelId,
  );

  const onJoin = () => {
    if (!allowed) return;
    joinVoice({ serverId: server.id, channelId, channelName });
  };

  return (
    <div className="flex flex-1 flex-col items-center justify-center p-8 text-center">
      <div className="w-full max-w-sm text-neutral-500">
        <p className="text-lg font-medium text-neutral-800">{channelName}</p>
        {occupants.length > 0 ? (
          <ul className="mt-4 flex flex-col gap-1 text-left text-sm text-neutral-700">
            {occupants.map(([id, flags]) => {
              const member = members.get(id);
              const pubs = voice.participants[id]?.pubs ?? [];
              const liveAudio =
                pubs.includes("a") && !flags.muted && !flags.deafened;
              return (
                <li
                  key={id}
                  className="flex h-10 items-center justify-between rounded-md bg-neutral-100 px-3"
                >
                  <span className="min-w-0 truncate font-medium">
                    {member?.name ?? "Mitglied"}
                  </span>
                  <span className="flex shrink-0 items-center gap-2">
                    <span className="w-10 text-right text-xs text-neutral-500">
                      {liveAudio ? "Audio" : "\u00a0"}
                    </span>
                    <VoiceStateIcons
                      inVoice
                      muted={flags.muted}
                      deafened={flags.deafened}
                      channelName={channelName}
                    />
                  </span>
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="mt-2 text-sm">Niemand ist in diesem Kanal.</p>
        )}
        {here ? (
          <div className="mt-5">
            <VoiceControls />
          </div>
        ) : allowed ? (
          <>
            <p className="mt-2 text-sm">
              Beitreten setzt dich sofort in den Kanal. Mute und Deafen gelten
              nur für diese Session.
            </p>
            <button
              type="button"
              onClick={onJoin}
              className="mt-5 rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700"
            >
              Beitreten
            </button>
          </>
        ) : (
          <p className="mt-2 text-sm">
            Du hast in diesem Server kein Recht, Voice beizutreten.
          </p>
        )}
      </div>
    </div>
  );
}
