import { describe, expect, it } from "vitest";

import type { Member, ServerDetail } from "../servers/types.ts";
import { dmCandidates, filterCandidates } from "./people.ts";
import type { DirectMessage } from "./types.ts";

const created_at = "2026-09-13T00:00:00Z";
const member = (user_id: string, name: string) =>
  ({ user_id, name, avatar_url: null }) as Member;
const server = (id: string, name: string, members: Member[]): ServerDetail => ({
  id,
  name,
  owner_id: "me",
  created_at,
  role: "member",
  permissions: [],
  member_permissions: [],
  categories: [],
  channels: [],
  members,
});
const dm = (id: string, peer: string): DirectMessage => ({
  id,
  kind: "dm",
  created_at,
  peer: { id: peer, name: peer, avatar_url: null },
});

const details = [
  server("s1", "Spieleabend", [
    member("me", "Rafi"),
    member("u-zoe", "Zoe"),
    member("u-mia", "Mia"),
  ]),
  server("s2", "Kochclub", [member("u-mia", "Mia"), member("u-ben", "Ben")]),
];

describe("new message candidates", () => {
  it("lists everyone the user shares a server with, once, without the user", () => {
    const people = dmCandidates(details, [], "me");
    expect(people.map((person) => person.name)).toEqual(["Ben", "Mia", "Zoe"]);
    expect(people.find((person) => person.id === "u-mia")?.servers).toEqual([
      "Spieleabend",
      "Kochclub",
    ]);
  });

  it("puts people already written to first, with their conversation", () => {
    const people = dmCandidates(details, [dm("d1", "u-zoe")], "me");
    expect(people.map((person) => person.name)).toEqual(["Zoe", "Ben", "Mia"]);
    expect(people[0]?.dmId).toBe("d1");
    expect(people[1]?.dmId).toBeNull();
  });

  it("filters by name, best match first", () => {
    const people = dmCandidates(details, [], "me");
    expect(filterCandidates(people, "").map((p) => p.name)).toEqual([
      "Ben",
      "Mia",
      "Zoe",
    ]);
    expect(filterCandidates(people, "mi").map((p) => p.name)).toEqual(["Mia"]);
    expect(filterCandidates(people, "e").map((p) => p.name)).toEqual([
      "Ben",
      "Zoe",
    ]);
  });
});
