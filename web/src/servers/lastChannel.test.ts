import { describe, expect, it } from "vitest";

import { railChannel } from "./lastChannel.ts";

describe("rail tile target", () => {
  it("returns to the channel last open", () => {
    expect(railChannel("c-memes", false)).toBe("c-memes");
  });

  it("opens the overview on a first visit and while a voice room is busy", () => {
    expect(railChannel(undefined, false)).toBeUndefined();
    expect(railChannel("c-memes", true)).toBeUndefined();
  });
});
