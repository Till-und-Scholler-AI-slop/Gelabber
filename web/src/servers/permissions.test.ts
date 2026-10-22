import { describe, expect, it } from "vitest";

import {
  ALL_PERMISSIONS,
  PERMISSIONS,
  can,
  normalisePermissions,
} from "./permissions.ts";

describe("permissions", () => {
  it("owner can do everything regardless of the member mask", () => {
    expect(can({ role: "owner", permissions: [] }, "manage_server")).toBe(true);
  });

  it("member is bound to the effective flags", () => {
    const member = {
      role: "member" as const,
      permissions: ["send_messages" as const],
    };
    expect(can(member, "send_messages")).toBe(true);
    expect(can(member, "manage_channels")).toBe(false);
    expect(can(undefined, "send_messages")).toBe(false);
  });

  it("normalises to the API's canonical order without duplicates", () => {
    expect(
      normalisePermissions(["go_live", "send_messages", "go_live"]),
    ).toEqual(["send_messages", "go_live"]);
    expect(normalisePermissions(ALL_PERMISSIONS)).toEqual(ALL_PERMISSIONS);
    expect(ALL_PERMISSIONS).toHaveLength(8);
    expect(ALL_PERMISSIONS).toContain("manage_messages");
  });

  it("keeps the go_live id and calls it Live gehen", () => {
    const live = PERMISSIONS.find((p) => p.id === "go_live");
    expect(live?.label).toBe("Live gehen");
    expect(live?.hint).toContain("Sprachkanal");
    expect(
      PERMISSIONS.map((p) => `${p.label} ${p.hint}`).join(" "),
    ).not.toMatch(/Go Live|Bildschirm teilen/);
  });
});
