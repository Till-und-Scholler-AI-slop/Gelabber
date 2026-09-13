import { beforeEach, describe, expect, it } from "vitest";

import { useLastDm } from "./lastDm.ts";

describe("last opened DM", () => {
  beforeEach(() => {
    useLastDm.setState({ channelId: null });
  });

  it("remembers the last channel and ignores a foreign forget", () => {
    useLastDm.getState().remember("dm-1");
    expect(useLastDm.getState().channelId).toBe("dm-1");
    useLastDm.getState().forget("dm-other");
    expect(useLastDm.getState().channelId).toBe("dm-1");
    useLastDm.getState().forget("dm-1");
    expect(useLastDm.getState().channelId).toBeNull();
  });
});
