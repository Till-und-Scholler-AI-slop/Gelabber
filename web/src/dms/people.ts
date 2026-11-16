// Who a new direct message can go to (#563): people already written to
// first, most recent first, then friends, then everyone else the user
// shares a server with, whose conversation starts as a message request.

import { matchScore } from "../components/quickSwitch.ts";
import type { FriendPerson } from "../friends/types.ts";
import type { ServerDetail } from "../servers/types.ts";
import type { DirectMessage } from "./types.ts";
import { compareNames } from "../i18n/format.ts";

export type DmCandidate = {
  id: string;
  name: string;
  avatar_url: string | null;
  /** The shared servers, by name. */
  servers: string[];
  /** The conversation with them, when there is one. */
  dmId: string | null;
  /** A friend: the conversation opens as an ordinary DM. */
  friend: boolean;
};

export function dmCandidates(
  details: readonly ServerDetail[],
  dms: readonly DirectMessage[],
  me: string | undefined,
  friends: readonly FriendPerson[] = [],
): DmCandidate[] {
  const people = new Map<string, DmCandidate>();
  const add = (
    id: string,
    name: string,
    avatar_url: string | null,
  ): DmCandidate => {
    const known = people.get(id);
    if (known) return known;
    const person: DmCandidate = {
      id,
      name,
      avatar_url,
      servers: [],
      dmId: null,
      friend: false,
    };
    people.set(id, person);
    return person;
  };
  for (const server of details)
    for (const member of server.members) {
      if (member.user_id === me) continue;
      add(member.user_id, member.name, member.avatar_url).servers.push(
        server.name,
      );
    }
  for (const friend of friends) {
    if (friend.id === me) continue;
    add(friend.id, friend.name, friend.avatar_url).friend = true;
  }
  const recent: DmCandidate[] = [];
  for (const dm of dms) {
    // An accepted DM stays open whatever the two share now.
    const person = add(dm.peer.id, dm.peer.name, dm.peer.avatar_url);
    person.dmId = dm.id;
    recent.push(person);
  }
  const rest = [...people.values()]
    .filter((person) => person.dmId === null)
    .sort(
      (a, b) =>
        Number(b.friend) - Number(a.friend) || compareNames(a.name, b.name),
    );
  return [...recent, ...rest];
}

/** The candidates whose name matches, best match first. */
export function filterCandidates(
  candidates: readonly DmCandidate[],
  query: string,
): DmCandidate[] {
  return candidates
    .flatMap((person, index) => {
      const score = matchScore(query, person.name);
      return score === null ? [] : [{ person, score, index }];
    })
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ person }) => person);
}
