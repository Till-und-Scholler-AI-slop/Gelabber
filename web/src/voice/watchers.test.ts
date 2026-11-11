import { describe, expect, it } from "vitest";

import type { VoiceFlags } from "./roster.ts";
import { watchersOf } from "./watchers.ts";

const in_ = (channelId: string): VoiceFlags => ({
  channelId,
  muted: false,
  deafened: false,
});

const occupancy: Record<string, VoiceFlags> = {
  jonas: in_("sofa"),
  lena: in_("sofa"),
  mira: in_("sofa"),
  tom: in_("sofa"),
  ute: in_("kueche"),
};

describe("watchersOf", () => {
  it("splits the channel's other occupants by the gateway's watches", () => {
    expect(
      watchersOf({
        owner: "jonas",
        channelId: "sofa",
        occupancy,
        // Ute watches from another channel and is not in this one.
        watches: { mira: ["jonas"], tom: ["anna"], ute: ["jonas"] },
      }),
    ).toEqual({ watching: ["mira"], notWatching: ["lena", "tom"] });
  });

  it("never lists the owner, even when the owner's set names them", () => {
    const { watching, notWatching } = watchersOf({
      owner: "jonas",
      channelId: "sofa",
      occupancy,
      watches: { jonas: ["jonas"] },
    });
    expect([...watching, ...notWatching]).not.toContain("jonas");
  });

  it("takes the own watch from sourceSubscriptions, not the echo", () => {
    const watches = { lena: [] as string[], mira: ["jonas"] };
    expect(
      watchersOf({
        owner: "jonas",
        channelId: "sofa",
        occupancy,
        watches,
        me: "lena",
        myWatches: { jonas: { s: true } },
      }).watching,
    ).toEqual(["lena", "mira"]);
    expect(
      watchersOf({
        owner: "jonas",
        channelId: "sofa",
        occupancy,
        watches: { lena: ["jonas"] },
        me: "lena",
        myWatches: { jonas: { s: false } },
      }).notWatching,
    ).toContain("lena");
    expect(
      watchersOf({
        owner: "jonas",
        channelId: "sofa",
        occupancy,
        watches: { lena: ["jonas"] },
        me: "lena",
      }).watching,
    ).toEqual([]);
  });
});
