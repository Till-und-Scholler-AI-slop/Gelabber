import { afterEach, describe, expect, it } from "vitest";

import {
  VOICE_ICON_SLOT_PX,
  applyVoiceJoin,
  applyVoiceLeave,
  applyVoiceMute,
  applyVoiceDeafen,
  applyVoiceSig,
  applyVoiceSnap,
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
});
