import { describe, expect, it } from "vitest";

import type { ReadState } from "../messages/readState.ts";
import { RAIL_DMS_MAX, focusAfterLeave, railDms } from "./railDms.ts";
import type { DirectMessage } from "./types.ts";

const NOW = Date.parse("2026-11-23T12:00:00Z");

const dm = (id: string): DirectMessage => ({
  id,
  kind: "dm",
  created_at: "2026-09-13T00:00:00Z",
  peer: { id: `peer-${id}`, name: `Peer ${id}`, avatar_url: null },
});

const row = (
  channel_id: string,
  unread_count: number,
  latest_order: number | null,
  extra: Partial<ReadState> = {},
): ReadState => ({
  channel_id,
  server_id: null,
  read_message_id: null,
  read_at: null,
  read_order: null,
  membership_at: "2026-09-13T00:00:00Z",
  latest_order,
  unread_count,
  mention_count: 0,
  ...extra,
});

const ids = (picked: ReturnType<typeof railDms>) =>
  picked.map(({ dm, count }) => `${dm.id}:${count}`);

describe("the rail's unread DMs", () => {
  it("shows DMs with unread messages, the newest message first", () => {
    const dms = [dm("a"), dm("b"), dm("c")];
    const rows = [row("a", 2, 10), row("b", 0, 30), row("c", 1, 20)];
    expect(ids(railDms(dms, rows, null, NOW))).toEqual(["c:1", "a:2"]);
  });

  it("keeps the list's order where no message order is known", () => {
    const dms = [dm("a"), dm("b"), dm("c")];
    const rows = [row("a", 1, null), row("b", 1, null), row("c", 1, 5)];
    expect(ids(railDms(dms, rows, null, NOW))).toEqual(["c:1", "a:1", "b:1"]);
  });

  it("leaves out the one being read, muted ones and those set to nothing", () => {
    const dms = [dm("a"), dm("b"), dm("c"), dm("d")];
    const rows = [
      row("a", 3, 4),
      row("b", 3, 3, { muted: true }),
      row("c", 3, 2, { level: "none" }),
      row("d", 3, 1, { muted: true, muted_until: "2026-11-23T11:00:00Z" }),
    ];
    expect(ids(railDms(dms, rows, "a", NOW))).toEqual(["d:3"]);
  });

  it("leaves out server channels and conversations not in the DM list", () => {
    const dms = [dm("a")];
    const rows = [
      row("a", 1, 1),
      row("request", 4, 9),
      row("a-server-channel", 7, 8, { server_id: "s1" }),
    ];
    expect(ids(railDms(dms, rows, null, NOW))).toEqual(["a:1"]);
  });

  it(`shows at most ${RAIL_DMS_MAX}`, () => {
    const dms = Array.from({ length: 8 }, (_, i) => dm(`d${i}`));
    const rows = dms.map((d, i) => row(d.id, 1, i));
    const picked = railDms(dms, rows, null, NOW);
    expect(picked).toHaveLength(RAIL_DMS_MAX);
    expect(picked[0]?.dm.id).toBe("d7");
    expect(railDms(dms, rows, null, NOW, 2)).toHaveLength(2);
  });

  it("shows nothing until both lists are there", () => {
    expect(railDms(undefined, [row("a", 1, 1)], null, NOW)).toEqual([]);
    expect(railDms([dm("a")], undefined, null, NOW)).toEqual([]);
  });
});

describe("the focus when a rail avatar leaves", () => {
  it("goes to the avatar now in its place, the last one, or the DM button", () => {
    expect(focusAfterLeave(1, 3)).toBe(1);
    expect(focusAfterLeave(2, 2)).toBe(1);
    expect(focusAfterLeave(0, 0)).toBeNull();
  });
});
