// Hint in text channels when someone is live. Local state first; the
// banner does not wait on the server.

import { Link } from "@tanstack/react-router";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useSession } from "../auth/session.ts";
import { EMPTY_LIVE, useVoiceRoster } from "./roster.ts";
import { stopWatching, useVoice, watchLive } from "./session.ts";

export function LiveHint({
  server,
  variant = "strip",
}: {
  server: ServerDetail;
  /** `pill`: compact, in the channel header on wide screens. */
  variant?: "strip" | "pill";
}) {
  const me = useSession((s) => s.user?.id);
  const lives = useVoiceRoster((s) => s.live[server.id] ?? EMPTY_LIVE);
  const voice = useVoice();
  const allowed = can(server, "join_voice");
  const names = new Map(server.members.map((m) => [m.user_id, m.name]));
  const channels = new Map(server.channels.map((c) => [c.id, c]));
  const entries = Object.entries(lives);
  if (entries.length === 0) return null;

  if (variant === "pill") {
    const [channelId, userId] = entries[0]!;
    const channel = channels.get(channelId);
    const who = userId === me ? "Du" : (names.get(userId) ?? "Jemand");
    const here = voice.status === "joined" && voice.channelId === channelId;
    const watching = voice.watching && voice.watchChannelId === channelId;
    const more = entries.length - 1;
    return (
      <div className="lr-live-pill">
        <span className="lr-live-badge">Live</span>
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: server.id, channelId }}
        >
          {who}
          {channel ? ` in ${channel.name}` : ""}
          {more > 0 ? ` +${more}` : ""}
        </Link>
        {!here && allowed && !watching && userId !== me ? (
          <button
            type="button"
            onClick={() =>
              watchLive({
                serverId: server.id,
                channelId,
                channelName: channel?.name ?? "Voice",
              })
            }
          >
            Zuschauen
          </button>
        ) : null}
      </div>
    );
  }

  return (
    <div className="lr-live-hint">
      {entries.map(([channelId, userId]) => {
        const channel = channels.get(channelId);
        const who = userId === me ? "Du" : (names.get(userId) ?? "Jemand");
        const here = voice.status === "joined" && voice.channelId === channelId;
        const watching = voice.watching && voice.watchChannelId === channelId;
        return (
          <div key={channelId}>
            <p>
              <span className="lr-live-badge">Live</span>
              {who} {userId === me ? "bist" : "ist"} live
              {channel ? (
                <>
                  {" "}
                  in{" "}
                  <Link
                    to="/s/$serverId/c/$channelId"
                    params={{ serverId: server.id, channelId }}
                  >
                    {channel.name}
                  </Link>
                </>
              ) : null}
              .
            </p>
            {!here && allowed && userId !== me ? (
              watching ? (
                <button
                  type="button"
                  onClick={() => stopWatching()}
                  className="is-quiet"
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
