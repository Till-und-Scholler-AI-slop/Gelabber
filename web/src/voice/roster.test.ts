import { afterEach, describe, expect, it } from "vitest";

import {
  EMPTY_LIVE,
  EMPTY_OCCUPANCY,
  VOICE_ICON_SLOT_PX,
  applyLiveEnd,
  applyLiveStart,
  applyVoiceJoin,
  applyVoiceLeave,
  applyVoiceMute,
  applyVoiceDeafen,
  applyVoiceSig,
  applyVoiceSnap,
  isLiveIn,
  liveIn,
  resetVoiceRoster,
  useVoiceRoster,
  voiceOf,
} from "./roster.ts";

afterEach(() => {
  resetVoiceRoster();
});

describe("voice roster", () => {
  it("tracks join/leave/mute/deafen without regrouping members", () => {
    applyVoiceJoin("s1", "u1", "voice");
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")).toEqual({
      channelId: "voice",
      muted: false,
      deafened: false,
    });
    applyVoiceMute("s1", "u1", true);
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")?.muted).toBe(
      true,
    );
    applyVoiceDeafen("s1", "u1", true);
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")).toEqual({
      channelId: "voice",
      muted: true,
      deafened: true,
    });
    applyVoiceLeave("s1", "u1");
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")).toBeNull();
    expect(VOICE_ICON_SLOT_PX).toBe(32);
  });

  it("tracks several people live in one channel, in the order they started", () => {
    const people = () => liveIn(useVoiceRoster.getState().live, "s1", "voice");
    const sig = (t: "p" | "u", u: string, k: "s" | "l" | "sa" | "v") =>
      applyVoiceSig({ op: "sig", t, s: "s1", c: "voice", u, k });
    sig("p", "u1", "s");
    sig("p", "u2", "s");
    // An older client's Go Live counts as live too.
    sig("p", "u3", "l");
    // Neither its sound nor a camera is a Live of its own.
    sig("p", "u4", "sa");
    sig("p", "u4", "v");
    expect(people()).toEqual(["u1", "u2", "u3"]);
    sig("u", "u1", "s");
    expect(people()).toEqual(["u2", "u3"]);
    sig("u", "u3", "l");
    expect(people()).toEqual(["u2"]);
    expect(isLiveIn(useVoiceRoster.getState().live.s1!, "u2")).toBe(true);
    expect(isLiveIn(useVoiceRoster.getState().live.s1!, "u1")).toBe(false);
  });

  it("keeps someone live while one of their two Lives runs", () => {
    applyLiveStart("s1", "voice", "u1", "s");
    applyLiveStart("s1", "voice", "u1", "l");
    applyLiveEnd("s1", "voice", "u1", "l");
    expect(liveIn(useVoiceRoster.getState().live, "s1", "voice")).toEqual([
      "u1",
    ]);
    applyLiveEnd("s1", "voice", "u1", "s");
    expect(useVoiceRoster.getState().live.s1).toEqual({});
  });

  it("clears live when the streamer leaves and restores it from a snap", () => {
    applyVoiceJoin("s1", "u1", "voice");
    applyLiveStart("s1", "voice", "u1", "s");
    applyVoiceLeave("s1", "u1");
    expect(liveIn(useVoiceRoster.getState().live, "s1", "voice")).toEqual([]);
    applyVoiceSnap("s1", [
      { u: "u2", c: "lounge", l: true },
      { u: "u3", c: "lounge", s: true },
      { u: "u4", c: "lounge" },
    ]);
    expect(liveIn(useVoiceRoster.getState().live, "s1", "lounge")).toEqual([
      "u2",
      "u3",
    ]);
  });

  it("ends a Live in the channel a leave names", () => {
    applyVoiceJoin("s1", "u1", "a");
    applyLiveStart("s1", "a", "u1", "s");
    // Already seen in the next channel when the leave of the first arrives.
    applyVoiceJoin("s1", "u1", "b");
    applyVoiceSig({ op: "sig", t: "l", s: "s1", c: "a", u: "u1" });
    expect(liveIn(useVoiceRoster.getState().live, "s1", "a")).toEqual([]);
  });

  it("replaces occupancy on a snapshot so reconnects drop ghosts", () => {
    applyVoiceJoin("s1", "u-old", "voice");
    applyVoiceSnap("s1", [{ u: "u2", c: "lounge", m: true }]);
    expect(
      voiceOf(useVoiceRoster.getState().byServer, "s1", "u-old"),
    ).toBeNull();
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u2")).toEqual({
      channelId: "lounge",
      muted: true,
      deafened: false,
    });
    applyVoiceSig({
      op: "sig",
      t: "r",
      s: "s1",
      snap: [],
    });
    expect(useVoiceRoster.getState().byServer.s1).toEqual({});
  });

  it("applies live sig frames, including join flags", () => {
    applyVoiceSig({
      op: "sig",
      t: "j",
      s: "s1",
      c: "voice",
      u: "u1",
      m: true,
    });
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")?.muted).toBe(
      true,
    );
    applyVoiceSig({
      op: "sig",
      t: "d",
      s: "s1",
      c: "voice",
      u: "u1",
      on: true,
    });
    expect(
      voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")?.deafened,
    ).toBe(true);
    applyVoiceSig({
      op: "sig",
      t: "l",
      s: "s1",
      c: "voice",
      u: "u1",
    });
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")).toBeNull();
  });

  it("empty occupancy and live fallbacks stay referentially stable", () => {
    const occupancy = useVoiceRoster.getState().byServer.missing ?? EMPTY_OCCUPANCY;
    const live = useVoiceRoster.getState().live.missing ?? EMPTY_LIVE;
    expect(occupancy).toBe(EMPTY_OCCUPANCY);
    expect(live).toBe(EMPTY_LIVE);
    expect(occupancy).toBe(
      useVoiceRoster.getState().byServer.missing ?? EMPTY_OCCUPANCY,
    );
    expect(live).toBe(useVoiceRoster.getState().live.missing ?? EMPTY_LIVE);
  });
});
