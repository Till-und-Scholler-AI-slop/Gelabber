// Who watches a Live, by name (#521): the roster's watches and the own
// watch from local state, for the caption's count and the call bar's.

import { useT, type TFunction } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
import { compareNames } from "../i18n/format.ts";
import { useUserId } from "../auth/scope.ts";
import type { Member } from "../servers/types.ts";
import { useVoice } from "./session.ts";
import {
  EMPTY_OCCUPANCY,
  EMPTY_WATCHES,
  useVoiceRoster,
  type VoiceFlags,
} from "./roster.ts";
import { watchersOf } from "./watchers.ts";

export type WatchPerson = { id: string; name: string; url: string | null };
export type LiveWatcherList = {
  watching: WatchPerson[];
  notWatching: WatchPerson[];
};

/** The people in `channelId` who watch `owner`'s Live, and the others there,
 * each sorted by name. The own watch is the local one. */
export function useLiveWatchers({
  serverId,
  channelId,
  owner,
  members,
}: {
  serverId: string;
  channelId: string;
  owner: string;
  members: readonly Member[] | undefined;
}): LiveWatcherList {
  const t = useT(voice);
  const me = useUserId();
  const occupancy = useVoiceRoster(
    (s) => s.byServer[serverId] ?? EMPTY_OCCUPANCY,
  );
  const watches = useVoiceRoster(
    (s) => s.watches[serverId]?.[channelId] ?? EMPTY_WATCHES,
  );
  const myWatches = useVoice((s) => s.sourceSubscriptions);
  const here = useVoice(
    (s) =>
      s.status === "joined" &&
      s.serverId === serverId &&
      s.channelId === channelId,
  );
  // The own join counts before the roster's echo, as on the stage.
  const people: Record<string, VoiceFlags> =
    here && me && occupancy[me]?.channelId !== channelId
      ? {
          ...occupancy,
          [me]: { channelId, muted: false, deafened: false },
        }
      : occupancy;
  const split = watchersOf({
    owner,
    channelId,
    occupancy: people,
    watches,
    me,
    myWatches,
  });
  const byId = new Map(
    (members ?? []).map((member) => [member.user_id, member]),
  );
  const person = (id: string): WatchPerson => {
    const member = byId.get(id);
    const name = member?.name ?? t("people.someone");
    return {
      id,
      name: id === me ? t("people.you", { name }) : name,
      url: member?.avatar_url ?? null,
    };
  };
  const sorted = (ids: string[]) =>
    ids.map(person).sort((a, b) => compareNames(a.name, b.name));
  return {
    watching: sorted(split.watching),
    notWatching: sorted(split.notWatching),
  };
}

/** "3 schauen zu", "1 schaut zu", "Niemand schaut zu". */
export function watchersTitle(t: TFunction<typeof voice.de>, count: number) {
  return count === 0 ? t("watchers.titleNone") : t("watchers.title", { count });
}
