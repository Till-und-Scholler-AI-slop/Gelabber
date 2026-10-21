import { describe, expect, it } from "vitest";

import { placeFromPath, startPlace, useLastPlace } from "./lastPlace.ts";

const servers = [{ id: "s1" }, { id: "s2" }];

describe("last place", () => {
  it("reads the place from workspace paths only", () => {
    expect(placeFromPath("/s/s2")).toEqual({ kind: "server", id: "s2" });
    expect(placeFromPath("/s/s2/c/c9")).toEqual({ kind: "server", id: "s2" });
    expect(placeFromPath("/d/d1")).toEqual({ kind: "dm", id: "d1" });
    expect(placeFromPath("/s/s2/settings")).toBeNull();
    expect(placeFromPath("/d")).toBeNull();
    expect(placeFromPath("/")).toBeNull();
    expect(placeFromPath("/invite/abc")).toBeNull();
  });

  it("reopens the remembered server while it is still listed", () => {
    expect(startPlace({ kind: "server", id: "s2" }, servers, [], true)).toEqual(
      {
        kind: "server",
        id: "s2",
      },
    );
    expect(
      startPlace({ kind: "server", id: "gone" }, servers, [], true),
    ).toEqual({
      kind: "server",
      id: "s1",
    });
    expect(startPlace(undefined, servers, undefined, false)).toEqual({
      kind: "server",
      id: "s1",
    });
  });

  it("waits for the DM list before giving up on a remembered DM", () => {
    const dm = { kind: "dm", id: "d1" } as const;
    expect(startPlace(dm, servers, undefined, false)).toBeUndefined();
    expect(startPlace(dm, servers, [{ id: "d1" }], true)).toEqual(dm);
    expect(startPlace(dm, servers, [], true)).toEqual({
      kind: "server",
      id: "s1",
    });
    expect(startPlace(dm, [], undefined, true)).toBeNull();
  });

  it("waits for the servers and has nowhere to go without any", () => {
    expect(startPlace(undefined, undefined, [], true)).toBeUndefined();
    expect(startPlace(undefined, [], [], true)).toBeNull();
  });

  it("keeps one place per account", () => {
    const { remember } = useLastPlace.getState();
    remember("u1", { kind: "server", id: "s1" });
    remember("u2", { kind: "dm", id: "d1" });
    const before = useLastPlace.getState().byUser;
    remember("u1", { kind: "server", id: "s1" });
    expect(useLastPlace.getState().byUser).toBe(before);
    expect(before).toEqual({
      u1: { kind: "server", id: "s1" },
      u2: { kind: "dm", id: "d1" },
    });
  });
});
