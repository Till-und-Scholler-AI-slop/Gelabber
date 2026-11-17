// The member list as one flat run of rows (#613): each presence group's
// heading, then its members. A virtual list mounts only the rows in view,
// so a server with thousands of members online keeps a few dozen in the
// page.

import type { PresenceGroup } from "../ws/live.ts";

export type MemberListRow<T> =
  | {
      kind: "heading";
      key: string;
      group: PresenceGroup;
      label: string;
      count: number;
      /** The list's first row: no gap above it. */
      first: boolean;
    }
  | {
      kind: "member";
      key: string;
      group: PresenceGroup;
      member: T;
      /** 1-based among the members, headings not counted: what a screen
       * reader announces as "3 of 1000". */
      position: number;
    };

export function memberListRows<T extends { user_id: string }>(
  groups: readonly { group: PresenceGroup; label: string; members: T[] }[],
): MemberListRow<T>[] {
  const rows: MemberListRow<T>[] = [];
  let position = 0;
  for (const { group, label, members } of groups) {
    rows.push({
      kind: "heading",
      key: `group:${group}`,
      group,
      label,
      count: members.length,
      first: rows.length === 0,
    });
    for (const member of members)
      rows.push({
        kind: "member",
        key: member.user_id,
        group,
        member,
        position: ++position,
      });
  }
  return rows;
}

/** Height guesses before a row is measured: the heading with the gap above
 * it, a member with a name only. Rows with a status or a voice channel are
 * taller and measured once mounted. */
export function memberRowEstimate(
  row: MemberListRow<unknown> | undefined,
): number {
  if (!row) return 0;
  if (row.kind === "heading") return row.first ? 31 : 39;
  return 44;
}
