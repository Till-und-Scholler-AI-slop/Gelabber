import { describe, expect, it } from "vitest";
import type { Message } from "../messages/types.ts";
import type { Channel, Member, ServerDetail } from "./types.ts";
import {
  messagePreview,
  overviewLine,
  overviewRoom,
  shortWhen,
  unreadChannels,
} from "./overview.ts";

const channels = [
  { id: "chat", name: "allgemein", kind: "text" },
  { id: "living", name: "Wohnzimmer", kind: "voice" },
  { id: "work", name: "Werkbank", kind: "voice" },
  { id: "tmp:voice", name: "Pending", kind: "voice" },
] as Channel[];
const members = [
  { user_id: "me", name: "Rafi" },
  { user_id: "mia", name: "Mia" },
  { user_id: "silas", name: "Silas" },
] as Member[];
const server = { id: "s", channels, members } as ServerDetail;
const idle = { status: "idle", serverId: null, channelId: null };
const roster = { mia: { channelId: "work", muted: false, deafened: false } };

describe("community overview", () => {
  it("prefers a current call only when it belongs to this server", () => {
    expect(
      overviewRoom(
        server,
        roster,
        { status: "joined", serverId: "s", channelId: "living" },
        "work",
      )?.id,
    ).toBe("living");
    expect(
      overviewRoom(server, roster, {
        status: "joined",
        serverId: "other",
        channelId: "living",
      })?.id,
    ).toBe("work");
  });
  it("uses a remembered room or actual occupancy, ignoring text, deleted, pending and stale users", () => {
    expect(overviewRoom(server, roster, idle, "living")?.id).toBe("living");
    for (const remembered of ["chat", "deleted", "tmp:voice"])
      expect(overviewRoom(server, roster, idle, remembered)?.id).toBe("work");
    expect(
      overviewRoom(
        server,
        { stranger: { channelId: "work", muted: false, deafened: false } },
        idle,
      )?.id,
    ).toBe("living");
    expect(
      overviewRoom({ ...server, channels: [channels[0]!] }, roster, idle),
    ).toBeUndefined();
  });
  it("says how many belong and how many are in a voice channel", () => {
    const joined = { status: "joined", serverId: "s", channelId: "living" };
    expect(overviewLine(server, {}, idle, "me")).toBe("3 Mitglieder");
    expect(overviewLine(server, roster, idle, "me")).toBe(
      "3 Mitglieder · 1 im Sprachkanal",
    );
    // The own call counts before the roster echo; a stale own entry does not.
    expect(
      overviewLine(
        server,
        {
          ...roster,
          me: roster.mia,
          silas: { ...roster.mia, channelId: "work" },
        },
        joined,
        "me",
      ),
    ).toBe("3 Mitglieder · 3 in Sprachkanälen");
    expect(overviewLine(server, { me: roster.mia }, idle, "me")).toBe(
      "3 Mitglieder",
    );
    // Strangers, text channels and other servers' calls are no one here.
    expect(
      overviewLine(
        server,
        {
          stranger: roster.mia,
          silas: { channelId: "chat", muted: false, deafened: false },
        },
        { ...joined, serverId: "other" },
        "me",
      ),
    ).toBe("3 Mitglieder");
    expect(
      overviewLine({ ...server, members: [members[0]!] }, {}, joined, "me"),
    ).toBe("1 Mitglied · 1 im Sprachkanal");
  });
  it("lists unread text channels, newest first, then in the sidebar's order", () => {
    const text = (id: string) =>
      ({ id, name: id, kind: "text", server_id: "s" }) as Channel;
    const list = [
      text("a"),
      text("b"),
      text("c"),
      text("d"),
      text("tmp:e"),
      channels[1]!,
    ];
    const badges = new Map([
      ["a", { count: 0, dot: true }],
      ["b", { count: 0, dot: false }],
      ["c", { count: 2, dot: true }],
      ["d", { count: 0, dot: true }],
      ["tmp:e", { count: 1, dot: true }],
      ["living", { count: 1, dot: true }],
    ]);
    const ids = (latest: Map<string, number | null>) =>
      unreadChannels(list, badges, latest).map((channel) => channel.id);
    expect(
      ids(
        new Map([
          ["a", 3],
          ["c", 9],
          ["d", null],
        ]),
      ),
    ).toEqual(["c", "a", "d"]);
    // An older API without the newest message keeps the sidebar's order.
    expect(ids(new Map())).toEqual(["a", "c", "d"]);
  });
  it("previews a message in one line, the way a reply quotes it", () => {
    const message = (change: Partial<Message>) =>
      ({
        author: { id: "mia", name: "Mia", avatar_url: null },
        content: "",
        attachments: [],
        ...change,
      }) as Message;
    const people = [
      { id: "00000000-0000-4000-8000-000000000001", name: "Rafi" },
    ];
    expect(
      messagePreview(
        message({
          content:
            "Hey <@00000000-0000-4000-8000-000000000001>,\n\nschau ||geheim|| an",
        }),
        people,
        "me",
      ),
    ).toEqual({ author: "Mia", text: "Hey @Rafi, schau ▒▒▒ an" });
    expect(
      messagePreview(
        message({
          author: { id: "me", name: "Rafi", avatar_url: null },
          gif: { slug: "x" } as Message["gif"],
        }),
        people,
        "me",
      ),
    ).toEqual({ author: "Du", text: "GIF" });
    expect(
      messagePreview(
        message({ attachments: [{ id: "f" }] as Message["attachments"] }),
        people,
        "me",
      ).text,
    ).toBe("Anhang");
    expect(
      messagePreview(
        message({
          system: { kind: "live_started", params: { voice_name: "Sofa" } },
        }),
        people,
        "me",
      ),
    ).toEqual({ author: null, text: "Mia ist live in Sofa." });
  });
  it("dates a message briefly", () => {
    const now = new Date(2026, 9, 29, 20, 0);
    expect(shortWhen(new Date(2026, 9, 29, 18, 28).toISOString(), now)).toBe(
      "18:28",
    );
    expect(shortWhen(new Date(2026, 9, 28, 23, 59).toISOString(), now)).toBe(
      "Gestern",
    );
    expect(shortWhen(new Date(2026, 9, 21, 9, 0).toISOString(), now)).toBe(
      "21.10.",
    );
    expect(shortWhen(new Date(2025, 11, 31, 9, 0).toISOString(), now)).toBe(
      "31.12.25",
    );
    expect(shortWhen("nonsense", now)).toBe("");
  });
});
