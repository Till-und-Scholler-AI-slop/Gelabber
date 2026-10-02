import { describe, expect, it } from "vitest";
import type { Channel, Member, ServerDetail } from "./types.ts";
import { overviewRoom, roomWelcome } from "./overview.ts";

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
  it("describes only known room occupants and excludes the current user", () => {
    const room = channels[2]!;
    expect(roomWelcome(members, roster, room, "me")).toBe(
      "Mia ist schon in „Werkbank“.",
    );
    expect(
      roomWelcome(
        members,
        { ...roster, silas: roster.mia, me: roster.mia, removed: roster.mia },
        room,
        "me",
      ),
    ).toBe("Mia und Silas sind schon in „Werkbank“.");
    expect(roomWelcome(members, roster, channels[1]!, "me")).toBe(
      "Mach es dir in „Wohnzimmer“ gemütlich.",
    );
  });
});
