// Voice channel body: join is a local state flip, ICE runs afterwards.

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { joinVoice, leaveVoice, useVoice } from "./session.ts";

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
  const here = voice.status === "joined" && voice.channelId === channelId;
  const members = new Map(server.members.map((member) => [member.user_id, member]));

  const onJoin = () => {
    if (!allowed) return;
    joinVoice({ serverId: server.id, channelId, channelName });
  };

  return (
    <div className="flex flex-1 flex-col items-center justify-center p-8 text-center">
      <div className="w-full max-w-sm text-neutral-500">
        <p className="text-lg font-medium text-neutral-800">{channelName}</p>
        {here ? (
          <>
            <p className="mt-2 text-sm text-neutral-800">Verbunden</p>
            <ul className="mt-4 flex flex-col gap-1 text-left text-sm text-neutral-700">
              {Object.keys(voice.participants).map((id) => {
                const member = members.get(id);
                const pubs = voice.participants[id]?.pubs ?? [];
                return (
                  <li
                    key={id}
                    className="flex items-center justify-between rounded-md bg-neutral-100 px-3 py-2"
                  >
                    <span className="truncate font-medium">
                      {member?.name ?? "Mitglied"}
                    </span>
                    <span className="text-xs text-neutral-500">
                      {pubs.includes("a") ? "Audio" : "…"}
                    </span>
                  </li>
                );
              })}
            </ul>
            <button
              type="button"
              onClick={() => leaveVoice()}
              className="mt-5 rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700"
            >
              Verlassen
            </button>
          </>
        ) : allowed ? (
          <>
            <p className="mt-2 text-sm">
              Beitreten setzt dich sofort in den Kanal. ICE läuft im
              Hintergrund — eigener SFU folgt, kein LiveKit.
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
