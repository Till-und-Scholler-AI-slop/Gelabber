// Hint in text channels when someone is live. Local state first; the
// banner does not wait on the server. A Live is watched in its voice
// channel, so the hint's action, "Zuschauen", goes there (watchLive.ts).

import { Link, useNavigate } from "@tanstack/react-router";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { useSession } from "../auth/session.ts";
import { T, useT } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import { liveSentence, nameList, youName } from "./roomSummary.ts";
import { EMPTY_LIVE, useVoiceRoster } from "./roster.ts";
import { useVoice } from "./session.ts";
import { currentCall, joinToWatch, watchLabel } from "./watchLive.ts";

export function LiveHint({
  server,
  variant = "strip",
}: {
  server: ServerDetail;
  /** `pill`: compact, in the channel header on wide screens. */
  variant?: "strip" | "pill";
}) {
  const t = useT(voice);
  const me = useSession((s) => s.user?.id);
  const lives = useVoiceRoster((s) => s.live[server.id] ?? EMPTY_LIVE);
  const call = useVoice(currentCall);
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
        ...(self ? [youName()] : []),
        ...ids
          .filter((id) => id !== me)
          .map((id) => names.get(id) ?? t("people.someone")),
      ],
      // Only someone else's Live is worth coming along for.
      others: ids.filter((id) => id !== me),
      here: call === `${server.id}/${channelId}`,
    };
  });
  const [first] = entries;
  if (!first) return null;

  /** Into the voice channel: its stage shows every Live there. Outside a
   * call this joins it and watches; in another call, the room asks before
   * it switches, so the button only opens the channel. */
  const come = (entry: (typeof entries)[number]) => {
    void navigate({
      to: "/s/$serverId/c/$channelId",
      params: { serverId: server.id, channelId: entry.channelId },
    });
    joinToWatch({
      serverId: server.id,
      channelId: entry.channelId,
      channelName: entry.name,
      people: entry.others,
    });
  };
  const action = (entry: (typeof entries)[number]) =>
    !entry.here && allowed && entry.others.length > 0 ? (
      <button type="button" onClick={() => come(entry)}>
        {watchLabel(call, server.id, entry.channelId)}
      </button>
    ) : null;

  if (variant === "pill") {
    const more = entries.length - 1;
    const sentence = liveSentence(first.who, first.self);
    const who = nameList(first.who);
    return (
      <div className="lr-live-pill">
        <span className="lr-live-badge">{t("live.badge")}</span>
        <Link
          to="/s/$serverId/c/$channelId"
          params={{ serverId: server.id, channelId: first.channelId }}
          title={
            first.name
              ? t("hint.in", { what: sentence, channel: first.name })
              : sentence
          }
        >
          {first.name ? t("hint.in", { what: who, channel: first.name }) : who}
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
            <span className="lr-live-badge">{t("live.badge")}</span>
            {entry.name ? (
              <T
                catalog={voice}
                k="hint.sentenceIn"
                params={{
                  sentence: liveSentence(entry.who, entry.self),
                  channel: entry.name,
                }}
                tags={{
                  link: (text) => (
                    <Link
                      to="/s/$serverId/c/$channelId"
                      params={{
                        serverId: server.id,
                        channelId: entry.channelId,
                      }}
                    >
                      {text}
                    </Link>
                  ),
                }}
              />
            ) : (
              t("hint.sentence", {
                sentence: liveSentence(entry.who, entry.self),
              })
            )}
          </p>
          {action(entry)}
        </div>
      ))}
    </div>
  );
}
