import { describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import { isGoneError, lastDmStillListed, shouldOpenLastDm } from "./open.ts";
import type { DirectMessage } from "./types.ts";

const dm = (id: string): DirectMessage => ({
  id,
  kind: "dm",
  created_at: "2026-09-13T00:00:00Z",
  peer: { id: "u2", name: "Bob", avatar_url: null },
});

describe("DM open targets", () => {
  it("treats not_found and forbidden as gone, not transport errors", () => {
    expect(isGoneError(new ApiError("not_found", 404, "gone"))).toBe(true);
    expect(isGoneError(new ApiError("forbidden", 403, "no"))).toBe(true);
    expect(isGoneError(new ApiError("network", 0, "offline"))).toBe(false);
    expect(isGoneError(new ApiError("timeout", 0, "slow"))).toBe(false);
    expect(isGoneError(new Error("boom"))).toBe(false);
  });

  it("only treats last-DM as live when it is in the current list", () => {
    expect(lastDmStillListed("dm-1", [dm("dm-1")])).toBe(true);
    expect(lastDmStillListed("dm-1", [dm("dm-2")])).toBe(false);
    expect(lastDmStillListed("dm-1", undefined)).toBe(false);
    expect(lastDmStillListed(null, [dm("dm-1")])).toBe(false);
  });

  it("does not auto-open last-DM after a failed detail fetch", () => {
    expect(shouldOpenLastDm("dm-1", [dm("dm-1")], false)).toBe(true);
    expect(shouldOpenLastDm("dm-1", [dm("dm-1")], true)).toBe(false);
    expect(shouldOpenLastDm("dm-1", [dm("other")], false)).toBe(false);
    expect(shouldOpenLastDm("dm-1", undefined, false)).toBe(false);
  });
});
