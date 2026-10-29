import { describe, expect, it } from "vitest";

import { CUE_LEVELS, renderCue, type CueName } from "./callSoundSynth.ts";

const names = Object.keys(CUE_LEVELS) as CueName[];

describe("call sound synth", () => {
  it("renders every cue the same way each time, finite, at its level", () => {
    for (const name of names) {
      const cue = renderCue(name, 24000);
      expect(renderCue(name, 24000), name).toEqual(cue);
      let peak = 0;
      for (const value of cue) {
        expect(Number.isFinite(value), name).toBe(true);
        peak = Math.max(peak, Math.abs(value));
      }
      // Float32 samples: the level to about seven digits.
      expect(peak, name).toBeCloseTo(CUE_LEVELS[name], 6);
    }
  });

  it("keeps every cue short and ends it in a fade", () => {
    for (const name of names) {
      const cue = renderCue(name, 24000);
      const seconds = cue.length / 24000;
      expect(seconds, name).toBeGreaterThanOrEqual(0.1);
      expect(seconds, name).toBeLessThanOrEqual(1.6);
      // The last sample is 1/480 of the way into the 20 ms fade.
      expect(Math.abs(cue[cue.length - 1]), name).toBeLessThan(0.002);
    }
    // Your own controls and the others' are quick taps.
    for (const name of ["muteSelf", "unmuteSelf", "muteOther", "unmuteOther"])
      expect(renderCue(name as CueName, 24000).length).toBeLessThan(
        0.35 * 24000,
      );
  });

  it("gives every event a sound of its own", () => {
    const cues = names.map((name) => renderCue(name, 24000));
    for (let a = 0; a < cues.length; a++)
      for (let b = a + 1; b < cues.length; b++)
        expect(cues[a], `${names[a]} vs ${names[b]}`).not.toEqual(cues[b]);
  });

  it("matches the audition it was picked from, at its 48 kHz", () => {
    // Lengths of the "Signatur" set on the audition page; the samples were
    // compared one by one when the set was ported.
    expect(
      Object.fromEntries(
        names.map((name) => [name, renderCue(name, 48000).length]),
      ),
    ).toEqual({
      joinSelf: 76800,
      joinOther: 24417,
      leaveOther: 60917,
      leaveSelf: 33611,
      muteSelf: 13645,
      unmuteSelf: 14701,
      muteOther: 6361,
      unmuteOther: 8952,
      deafen: 27531,
      undeafen: 26983,
      liveStart: 76800,
      liveEnd: 76800,
      message: 76800,
      viewerJoin: 75138,
      viewerLeave: 58181,
      cameraOn: 10459,
      cameraOff: 10771,
    });
  });
});
