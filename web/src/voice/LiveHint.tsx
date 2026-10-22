// Hint in text channels when someone is live. Local state first; the
// banner does not wait on the server. A Live is watched in its voice
// channel, so the hint's action is to come along.

import { Link, useNavigate } from "@tanstack/react-router";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useSession } from "../auth/session.ts";
import { liveSentence, nameList } from "./roomSummary.ts";
import { EMPTY_LIVE, useVoiceRoster } from "./roster.ts";
import { joinVoice, useVoice } from "./session.ts";

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
  const call = useVoice((s) =>
    s.status === "joined" ? `${s.serverId}/${s.channelId}` : null,
  );
  const navigate = useNavigate();
  const allowed = can(server, "join_voice");
  const names = new Map(server.members.map((m) => [m.user_id, m.name]));
  const channels = new Map(server.channels.map((c) => [c.id, c]));
  const entries = Object.entries(lives).map(([channelId, people]) => {
    const ids = Object.keys(people);
    const self = me !== undefined && ids.includes(me);
    return {
      channelId,
      name: channels.get(channelId)?.name ?? null,
      self,
      // You first, as German says it.
      who: [
        ...(self ? ["Du"] : []),
        ...ids.filter((id) => id !== me).map((id) => names.get(id) ?? "Jemand"),
      ],
      // Only someone else's Live is worth coming along for.
      others: ids.some((id) => id !== me),
      here: call === `${server.id}/${channelId}`,
    };
  });
  const [first] = entries;
  if (!first) return null;

  /** Into the voice channel: its stage shows every Live there. In another
   * call, the room asks before it switches. */
  const come = (channelId: string, channelName: string | null) => {
    void navigate({
      to: "/s/$serverId/c/$channelId",
      params: { serverId: server.id, channelId },
    });
    if (call === null)
      joinVoice({
        serverId: server.id,
        channelId,
        channelName: channelName ?? "Sprachkanal",
      });
  };
  const action = (entry: (typeof entries)[number]) =>
    !entry.here && allowed && entry.others ? (
      <button type="button" onClick={() => come(entry.channelId, entry.name)}>
        {call === null ? "Dazukommen" : "Zum Kanal"}
      </button>
    ) : null;

  if (variant === "pill") {
    const more = entries.length - 1;
    const where = first.name ? ` in ${first.name}` : "";
    return (
      <div className="lr-live-pill">
        <span className="lr-live-badge">Live</span>
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: server.id, channelId: first.channelId }}
          title={`${liveSentence(first.who, first.self)}${where}`}
        >
          {nameList(first.who)}
          {where}
          {more > 0 ? ` +${more}` : ""}
        </Link>
        {action(first)}
      </div>
    );
  }

  return (
    <div className="lr-live-hint">
      {entries.map((entry) => (
        <div key={entry.channelId}>
          <p>
            <span className="lr-live-badge">Live</span>
            {liveSentence(entry.who, entry.self)}
            {entry.name ? (
              <>
                {" "}
                in{" "}
                <Link
                  to="/s/$serverId/c/$channelId"
                  params={{ serverId: server.id, channelId: entry.channelId }}
                >
                  {entry.name}
                </Link>
              </>
            ) : null}
            .
          </p>
          {action(entry)}
        </div>
      ))}
    </div>
  );
}
