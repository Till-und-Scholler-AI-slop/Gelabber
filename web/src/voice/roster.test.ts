import { afterEach, describe, expect, it } from "vitest";

import {
  EMPTY_LIVE,
  EMPTY_OCCUPANCY,
  EMPTY_WATCHES,
  VOICE_ICON_SLOT_PX,
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
    const sig = (t: "p" | "u", u: string, k: "s" | "sa" | "v") =>
      applyVoiceSig({ op: "sig", t, s: "s1", c: "voice", u, k });
    sig("p", "u1", "s");
    sig("p", "u2", "s");
    sig("p", "u3", "s");
    // Neither its sound nor a camera is a Live of its own.
    sig("p", "u4", "sa");
    sig("p", "u4", "v");
    expect(people()).toEqual(["u1", "u2", "u3"]);
    sig("u", "u1", "s");
    expect(people()).toEqual(["u2", "u3"]);
    sig("u", "u3", "s");
    expect(people()).toEqual(["u2"]);
    expect(isLiveIn(useVoiceRoster.getState().live.s1!, "u2")).toBe(true);
    expect(isLiveIn(useVoiceRoster.getState().live.s1!, "u1")).toBe(false);
  });

  it("clears live when the streamer leaves and restores it from a snap", () => {
    applyVoiceJoin("s1", "u1", "voice");
    applyLiveStart("s1", "voice", "u1", "s");
    applyVoiceLeave("s1", "u1");
    expect(liveIn(useVoiceRoster.getState().live, "s1", "voice")).toEqual([]);
    applyVoiceSnap("s1", [
      { u: "u2", c: "lounge", s: true },
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

  it("keeps someone where they are when another channel's leave comes", () => {
    // Live in b; a second device's seat in a leaves.
    applyVoiceJoin("s1", "u1", "b");
    applyLiveStart("s1", "b", "u1", "s");
    applyLiveStart("s1", "a", "u1", "s");
    applyVoiceSig({ op: "sig", t: "l", s: "s1", c: "a", u: "u1" });
    const state = useVoiceRoster.getState();
    expect(liveIn(state.live, "s1", "a")).toEqual([]);
    expect(liveIn(state.live, "s1", "b")).toEqual(["u1"]);
    expect(voiceOf(state.byServer, "s1", "u1")?.channelId).toBe("b");
    // Their own channel's leave still takes them out.
    applyVoiceSig({ op: "sig", t: "l", s: "s1", c: "b", u: "u1" });
    expect(liveIn(useVoiceRoster.getState().live, "s1", "b")).toEqual([]);
    expect(voiceOf(useVoiceRoster.getState().byServer, "s1", "u1")).toBeNull();
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
    const occupancy =
      useVoiceRoster.getState().byServer.missing ?? EMPTY_OCCUPANCY;
    const live = useVoiceRoster.getState().live.missing ?? EMPTY_LIVE;
    expect(occupancy).toBe(EMPTY_OCCUPANCY);
    expect(live).toBe(EMPTY_LIVE);
    expect(occupancy).toBe(
      useVoiceRoster.getState().byServer.missing ?? EMPTY_OCCUPANCY,
    );
    expect(live).toBe(useVoiceRoster.getState().live.missing ?? EMPTY_LIVE);
  });

  it("replaces someone's watches with the set a w frame carries", () => {
    const watches = () => useVoiceRoster.getState().watches.s1 ?? EMPTY_WATCHES;
    const w = (u: string, owners?: string[]) =>
      applyVoiceSig({ op: "sig", t: "w", s: "s1", c: "voice", u, w: owners });
    w("u2", ["u1"]);
    w("u3", ["u1", "u4"]);
    expect(watches()).toEqual({ u2: ["u1"], u3: ["u1", "u4"] });
    w("u3", ["u4"]);
    expect(watches().u3).toEqual(["u4"]);
    w("u2", []);
    expect(watches().u2).toEqual([]);
    // A frame without a set means none.
    w("u3");
    expect(watches().u3).toEqual([]);
    // The same set again keeps the state.
    const before = useVoiceRoster.getState();
    w("u3", []);
    expect(useVoiceRoster.getState()).toBe(before);
  });

  it("drops a watcher's set when they leave or a snapshot lacks them", () => {
    applyVoiceJoin("s1", "u2", "voice");
    applyVoiceJoin("s1", "u3", "voice");
    applyVoiceSig({
      op: "sig",
      t: "w",
      s: "s1",
      c: "voice",
      u: "u2",
      w: ["u1"],
    });
    applyVoiceSig({
      op: "sig",
      t: "w",
      s: "s1",
      c: "voice",
      u: "u3",
      w: ["u1"],
    });
    applyVoiceSig({ op: "sig", t: "l", s: "s1", c: "voice", u: "u2" });
    expect(useVoiceRoster.getState().watches.s1).toEqual({ u3: ["u1"] });
    applyVoiceSig({
      op: "sig",
      t: "w",
      s: "s1",
      c: "voice",
      u: "u5",
      w: ["u1"],
    });
    applyVoiceSig({
      op: "sig",
      t: "r",
      s: "s1",
      snap: [{ u: "u3", c: "voice" }],
    });
    expect(useVoiceRoster.getState().watches.s1).toEqual({ u3: ["u1"] });
    resetVoiceRoster();
    expect(useVoiceRoster.getState().watches).toEqual({});
  });

  it("keeps watches when the leave is of a seat in another channel", () => {
    applyVoiceJoin("s1", "u2", "b");
    applyVoiceSig({ op: "sig", t: "w", s: "s1", c: "b", u: "u2", w: ["u1"] });
    applyVoiceSig({ op: "sig", t: "l", s: "s1", c: "a", u: "u2" });
    expect(useVoiceRoster.getState().watches.s1).toEqual({ u2: ["u1"] });
  });
});
