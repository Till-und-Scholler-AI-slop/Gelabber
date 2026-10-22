// Who a new direct message can go to: everyone the user shares a server
// with. People already written to come first, most recent first.

import { matchScore } from "../components/quickSwitch.ts";
import type { ServerDetail } from "../servers/types.ts";
import type { DirectMessage } from "./types.ts";

export type DmCandidate = {
  id: string;
  name: string;
  avatar_url: string | null;
  /** The shared servers, by name. */
  servers: string[];
  /** The conversation with them, when there is one. */
  dmId: string | null;
};

export function dmCandidates(
  details: readonly ServerDetail[],
  dms: readonly DirectMessage[],
  me: string | undefined,
): DmCandidate[] {
  const people = new Map<string, DmCandidate>();
  for (const server of details)
    for (const member of server.members) {
      if (member.user_id === me) continue;
      const known = people.get(member.user_id);
      if (known) known.servers.push(server.name);
      else
        people.set(member.user_id, {
          id: member.user_id,
          name: member.name,
          avatar_url: member.avatar_url,
          servers: [server.name],
          dmId: null,
        });
    }
  const recent: DmCandidate[] = [];
  for (const dm of dms) {
    const person = people.get(dm.peer.id);
    if (!person) continue;
    person.dmId = dm.id;
    recent.push(person);
  }
  const rest = [...people.values()]
    .filter((person) => person.dmId === null)
    .sort((a, b) => a.name.localeCompare(b.name, "de"));
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
