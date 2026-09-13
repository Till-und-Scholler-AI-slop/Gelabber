// Hint in text channels when someone is live. Local state first; the
// banner does not wait on the server.

import { Link } from "@tanstack/react-router";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useSession } from "../auth/session.ts";
import { useVoiceRoster } from "./roster.ts";
import { stopWatching, useVoice, watchLive } from "./session.ts";

export function LiveHint({ server }: { server: ServerDetail }) {
  const me = useSession((s) => s.user?.id);
  const lives = useVoiceRoster((s) => s.live[server.id] ?? {});
  const voice = useVoice();
  const allowed = can(server, "join_voice");
  const names = new Map(server.members.map((m) => [m.user_id, m.name]));
  const channels = new Map(server.channels.map((c) => [c.id, c]));
  const entries = Object.entries(lives);
  if (entries.length === 0) return null;

  return (
    <div className="shrink-0 border-b border-red-100 bg-red-50 px-4 py-2 text-sm text-red-800">
      {entries.map(([channelId, userId]) => {
        const channel = channels.get(channelId);
        const who = userId === me ? "Du" : (names.get(userId) ?? "Jemand");
        const here =
          voice.status === "joined" && voice.channelId === channelId;
        const watching = voice.watching && voice.watchChannelId === channelId;
        return (
          <div
            key={channelId}
            className="flex flex-wrap items-center justify-between gap-2"
          >
            <p>
              <span className="mr-1.5 rounded bg-red-600 px-1 py-px text-[10px] font-semibold tracking-wide text-white uppercase">
                Live
              </span>
              {who} ist live
              {channel ? (
                <>
                  {" "}
                  in{" "}
                  <Link
                    to="/s/$serverId/c/$channelId"
                    params={{ serverId: server.id, channelId }}
                    className="font-medium underline decoration-red-300 underline-offset-2"
                  >
                    {channel.name}
                  </Link>
                </>
              ) : null}
              .
            </p>
            {!here && allowed ? (
              watching ? (
                <button
                  type="button"
                  onClick={() => stopWatching()}
                  className="text-xs font-medium text-red-700 hover:underline"
                >
                  Nicht mehr zuschauen
                </button>
              ) : (
                <button
                  type="button"
                  onClick={() =>
                    watchLive({
                      serverId: server.id,
                      channelId,
                      channelName: channel?.name ?? "Voice",
                    })
                  }
                  className="text-xs font-medium text-red-700 hover:underline"
                >
                  Zuschauen
                </button>
              )
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
