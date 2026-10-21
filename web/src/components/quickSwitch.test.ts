import { describe, expect, it } from "vitest";

import type { DirectMessage } from "../dms/types.ts";
import type { Badge } from "../messages/notifyLevel.ts";
import type { Channel, ServerDetail } from "../servers/types.ts";
import {
  matchScore,
  rankEntries,
  switchEntries,
  type SwitchEntry,
} from "./quickSwitch.ts";

const created_at = "2026-09-13T00:00:00Z";
const channel = (id: string, kind: Channel["kind"] = "text"): Channel => ({
  id,
  server_id: "s1",
  category_id: null,
  name: id,
  kind,
  created_at,
});
const server: ServerDetail = {
  id: "s1",
  name: "Spieleabend",
  owner_id: "u1",
  created_at,
  role: "member",
  permissions: [],
  member_permissions: [],
  categories: [],
  channels: [channel("allgemein"), channel("wohnzimmer", "voice")],
  members: [],
};
const dm: DirectMessage = {
  id: "d1",
  kind: "dm",
  created_at,
  peer: { id: "u2", name: "Mia", avatar_url: null },
};

const none: Badge = { count: 0, dot: false };
const entry = (label: string, badge: Badge = none): SwitchEntry => ({
  id: label,
  kind: "text",
  label,
  detail: null,
  target: { kind: "channel", serverId: "s1", channelId: label },
  badge,
});

describe("quick switcher entries", () => {
  it("offers direct messages, channels and servers", () => {
    const entries = switchEntries({
      servers: [server],
      details: [server],
      dms: [dm],
      badges: new Map([["allgemein", { count: 2, dot: true }]]),
      current: undefined,
    });
    expect(entries.map((item) => item.id)).toEqual([
      "dm:d1",
      "channel:allgemein",
      "channel:wohnzimmer",
      "server:s1",
    ]);
    expect(entries[1]).toMatchObject({
      kind: "text",
      detail: "Spieleabend",
      badge: { count: 2, dot: true },
    });
    // The server sums up its channels.
    expect(entries[3]?.badge).toEqual({ count: 2, dot: true });
  });

  it("leaves out the conversation open now", () => {
    const entries = switchEntries({
      servers: [server],
      details: [server],
      dms: [dm],
      badges: new Map(),
      current: "allgemein",
    });
    expect(entries.map((item) => item.id)).not.toContain("channel:allgemein");
  });

  it("lists a server whose channels are not loaded yet", () => {
    const entries = switchEntries({
      servers: [server],
      details: [],
      dms: [],
      badges: new Map(),
      current: undefined,
    });
    expect(entries.map((item) => item.id)).toEqual(["server:s1"]);
  });
});

describe("quick switcher matching", () => {
  it("prefers the start of the name, then a word, then anywhere", () => {
    expect(matchScore("all", "allgemein")).toBe(4);
    expect(matchScore("abend", "Spiele abend")).toBe(3);
    expect(matchScore("gem", "allgemein")).toBe(2);
    expect(matchScore("agm", "allgemein")).toBe(1);
    expect(matchScore("xyz", "allgemein")).toBeNull();
  });

  it("ignores case, accents and a leading # or @", () => {
    expect(matchScore("#ALL", "allgemein")).toBe(4);
    expect(matchScore("@mia", "Mia")).toBe(4);
    expect(matchScore("kuche", "Küche")).toBe(4);
  });

  it("puts the best match first and unread ones first among equals", () => {
    const entries = [
      entry("allgemein"),
      entry("ankündigungen", { count: 0, dot: true }),
      entry("spiele-alle"),
    ];
    expect(rankEntries(entries, "a").map((item) => item.label)).toEqual([
      "ankündigungen",
      "allgemein",
      "spiele-alle",
    ]);
  });

  it("lists what is new first when nothing is typed", () => {
    const entries = [
      entry("ruhig"),
      entry("neu", { count: 0, dot: true }),
      entry("erwähnt", { count: 3, dot: true }),
    ];
    expect(rankEntries(entries, "").map((item) => item.label)).toEqual([
      "erwähnt",
      "neu",
      "ruhig",
    ]);
    expect(rankEntries(entries, "", 2)).toHaveLength(2);
  });
});
