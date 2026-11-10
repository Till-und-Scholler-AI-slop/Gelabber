import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import { ALL_PERMISSIONS } from "./permissions.ts";
import {
  BASE_ROLE,
  ROLE_COLORS,
  accessOf,
  accessSummary,
  effectivePermissions,
  inheritedFromBase,
  nextColor,
  nextRoleName,
  roleCounts,
  rolesOf,
  seesChannel,
  toggleAccess,
} from "./roles.ts";
import type {
  Channel,
  Member,
  RoleColor,
  ServerDetail,
  ServerRole,
} from "./types.ts";

const created_at = "2026-11-01T00:00:00Z";

function role(
  id: string,
  name: string,
  position: number,
  color: RoleColor = "blue",
  permissions: ServerRole["permissions"] = [],
): ServerRole {
  return {
    id,
    server_id: "s1",
    name,
    color,
    permissions,
    position,
    created_at,
  };
}

function member(
  user_id: string,
  role_ids: string[] = [],
  memberRole: Member["role"] = "member",
): Member {
  return {
    user_id,
    name: user_id,
    avatar_url: null,
    joined_at: created_at,
    role: memberRole,
    role_ids,
  };
}

function channel(limited: boolean, role_ids: string[] = []): Channel {
  return {
    id: "spiele",
    server_id: "s1",
    category_id: null,
    name: "spiele",
    kind: "text",
    created_at,
    limited,
    role_ids,
  };
}

// As roles-editor.png: Moderation 1, Stammgäste 3, everyone 5.
const moderation = role("r-mod", "Moderation", 0, "blue", [
  "moderate_members",
  "manage_messages",
  "mention_everyone",
]);
const regulars = role("r-reg", "Stammgäste", 1, "green");
const server: ServerDetail = {
  id: "s1",
  name: "Spieleabend",
  owner_id: "lena",
  created_at,
  role: "owner",
  permissions: ALL_PERMISSIONS,
  member_permissions: ["send_messages", "send_files", "join_voice", "go_live"],
  categories: [],
  channels: [],
  // Out of position order on purpose: helpers sort by position.
  roles: [regulars, moderation],
  members: [
    member("lena", [], "owner"),
    member("jonas", ["r-reg", "r-mod"]),
    member("mira", ["r-reg"]),
    member("tom", ["r-reg"]),
    member("ada"),
  ],
};

afterEach(() => setLocale("de"));

describe("roleCounts", () => {
  it("counts the owner once, everyone for the base role and holders per role", () => {
    expect(roleCounts(server)).toEqual({
      owner: 1,
      base: 5,
      roles: { "r-mod": 1, "r-reg": 3 },
    });
  });

  it("gives a role nobody holds a zero and ignores unknown ids", () => {
    const counts = roleCounts({
      roles: [role("r-new", "Neu", 2)],
      members: [member("ada", ["r-gone", "r-gone"])],
    });
    expect(counts.roles).toEqual({ "r-new": 0 });
  });
});

describe("rolesOf", () => {
  it("lists a member's roles in position order", () => {
    expect(rolesOf(member("jonas", ["r-reg", "r-mod"]), server)).toEqual([
      moderation,
      regulars,
    ]);
    expect(rolesOf(member("ada"), server)).toEqual([]);
    expect(rolesOf(member("x", ["r-gone"]), server)).toEqual([]);
  });
});

describe("seesChannel", () => {
  const limited = channel(true, ["r-mod"]);

  it("lets everyone see a channel that is not limited", () => {
    expect(seesChannel(member("ada"), channel(false), server)).toBe(true);
  });

  it("needs one of the channel's roles when it is limited", () => {
    expect(seesChannel(member("jonas", ["r-mod"]), limited, server)).toBe(true);
    expect(seesChannel(member("mira", ["r-reg"]), limited, server)).toBe(false);
    expect(seesChannel(member("ada"), limited, server)).toBe(false);
  });

  it("always lets the owner see it, even without roles", () => {
    expect(seesChannel(member("lena", [], "owner"), limited, server)).toBe(
      true,
    );
    expect(seesChannel(member("lena"), channel(true), server)).toBe(true);
    expect(seesChannel(member("jonas", ["r-mod"]), channel(true), server)).toBe(
      false,
    );
  });
});

describe("accessSummary", () => {
  it("reads Alle Mitglieder, the role names in position order, or Nur Inhaber", () => {
    expect(accessSummary(channel(false), server)).toBe("Alle Mitglieder");
    expect(accessSummary(channel(true, ["r-reg", "r-mod"]), server)).toBe(
      "Moderation, Stammgäste",
    );
    expect(accessSummary(channel(true), server)).toBe("Nur Inhaber");
    expect(accessSummary(channel(true, ["r-gone"]), server)).toBe(
      "Nur Inhaber",
    );
  });

  it("follows the language", () => {
    setLocale("en");
    expect(accessSummary(channel(false), server)).toBe("All members");
    expect(accessSummary(channel(true), server)).toBe("Owner only");
  });
});

describe("toggleAccess", () => {
  it("reads a channel's selection", () => {
    expect(accessOf(channel(false))).toBeNull();
    expect(accessOf(channel(true, ["r-mod"]))).toEqual(["r-mod"]);
    expect(accessOf(channel(true))).toEqual([]);
  });

  it("clears Alle Mitglieder when a role is chosen", () => {
    expect(toggleAccess(null, "r-mod")).toEqual(["r-mod"]);
    expect(toggleAccess(["r-mod"], "r-reg")).toEqual(["r-mod", "r-reg"]);
  });

  it("clears the roles when Alle Mitglieder is chosen", () => {
    expect(toggleAccess(["r-mod", "r-reg"], BASE_ROLE)).toBeNull();
    expect(toggleAccess([], BASE_ROLE)).toBeNull();
    expect(toggleAccess(null, BASE_ROLE)).toBeNull();
  });

  it("selects Alle Mitglieder again when the last role is unchecked", () => {
    expect(toggleAccess(["r-mod", "r-reg"], "r-mod")).toEqual(["r-reg"]);
    expect(toggleAccess(["r-reg"], "r-reg")).toBeNull();
  });

  it("adds a role to an owner-only selection", () => {
    expect(toggleAccess([], "r-mod")).toEqual(["r-mod"]);
  });
});

describe("inheritedFromBase", () => {
  it("is true for the flags Alle Mitglieder holds", () => {
    expect(inheritedFromBase(server, "send_messages")).toBe(true);
    expect(inheritedFromBase(server, "moderate_members")).toBe(false);
  });
});

describe("nextColor", () => {
  it("takes the first colour no role uses", () => {
    expect(nextColor({ roles: [] })).toBe("amber");
    expect(nextColor(server)).toBe("amber");
    expect(
      nextColor({
        roles: [role("a", "A", 0, "amber"), role("b", "B", 1, "blue")],
      }),
    ).toBe("green");
  });

  it("falls back to amber when all six are in use", () => {
    expect(
      nextColor({
        roles: ROLE_COLORS.map((color, i) => role(color, color, i, color)),
      }),
    ).toBe("amber");
  });
});

describe("nextRoleName", () => {
  it("numbers the name when a role has it, ignoring case", () => {
    expect(nextRoleName(server, "Neue Rolle")).toBe("Neue Rolle");
    const taken = {
      roles: [role("a", "neue rolle", 0), role("b", "Neue Rolle 2", 1)],
    };
    expect(nextRoleName(taken, "Neue Rolle")).toBe("Neue Rolle 3");
  });
});

describe("effectivePermissions", () => {
  it("is the union of the base role and the member's roles, in canonical order", () => {
    expect(effectivePermissions(member("jonas", ["r-mod"]), server)).toEqual([
      "moderate_members",
      "manage_messages",
      "mention_everyone",
      "send_messages",
      "send_files",
      "join_voice",
      "go_live",
    ]);
    expect(effectivePermissions(member("ada"), server)).toEqual(
      server.member_permissions,
    );
  });

  it("is everything for the owner", () => {
    expect(effectivePermissions(member("lena", [], "owner"), server)).toEqual(
      ALL_PERMISSIONS,
    );
  });
});
