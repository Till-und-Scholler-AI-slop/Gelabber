import { beforeEach, describe, expect, it } from "vitest";

import { lastDmId, useLastDm } from "./lastDm.ts";

describe("last opened DM", () => {
  beforeEach(() => {
    useLastDm.setState({ byUser: {} });
  });

  it("remembers the last channel per user and ignores a foreign forget", () => {
    useLastDm.getState().remember("ada", "dm-1");
    expect(lastDmId(useLastDm.getState().byUser, "ada")).toBe("dm-1");
    expect(lastDmId(useLastDm.getState().byUser, "bob")).toBeNull();
    useLastDm.getState().forget("ada", "dm-other");
    expect(lastDmId(useLastDm.getState().byUser, "ada")).toBe("dm-1");
    useLastDm.getState().forget("ada", "dm-1");
    expect(lastDmId(useLastDm.getState().byUser, "ada")).toBeNull();
  });
});
