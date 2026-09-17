import { describe, expect, it } from "vitest";

import { workspaceTopics } from "./useGateway.ts";

describe("workspace subscribe set", () => {
  it("subscribes a server and its channel", () => {
    expect(workspaceTopics("srv", "ch")).toEqual([
      { s: "srv" },
      { s: "srv", c: "ch" },
    ]);
  });

  it("uses the DM channel id as protocol s", () => {
    expect(workspaceTopics(undefined, "dm-1")).toEqual([
      { s: "dm-1", c: "dm-1" },
    ]);
  });

  it("merges extra channel topics without duplicates", () => {
    expect(
      workspaceTopics("srv", "ch", [
        { s: "srv", c: "ch" },
        { s: "srv", c: "other" },
        { s: "dm-1", c: "dm-1" },
      ]),
    ).toEqual([
      { s: "srv" },
      { s: "srv", c: "ch" },
      { s: "srv", c: "other" },
      { s: "dm-1", c: "dm-1" },
    ]);
  });
});
