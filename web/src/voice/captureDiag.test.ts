import { afterEach, describe, expect, it, vi } from "vitest";

import {
  compareCaptureSettings,
  isCaptureDiagEnabled,
  logCaptureSettings,
} from "./captureDiag.ts";

describe("compareCaptureSettings", () => {
  it("compares requested keys against fake getSettings values", () => {
    const requested: MediaTrackConstraints = {
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
    };
    const applied: MediaTrackSettings = {
      echoCancellation: true,
      noiseSuppression: false,
      autoGainControl: true,
      channelCount: 1,
      sampleRate: 48000,
    };
    const comparison = compareCaptureSettings(requested, applied);
    expect(comparison.echoCancellation).toEqual({
      requested: true,
      applied: true,
      match: true,
    });
    expect(comparison.noiseSuppression).toEqual({
      requested: true,
      applied: false,
      match: false,
    });
    expect(comparison.autoGainControl?.match).toBe(true);
    expect(comparison.channelCount?.match).toBe(true);
    expect(comparison.sampleRate).toBeUndefined();
  });

  it("includes sampleRate only when requested", () => {
    const comparison = compareCaptureSettings(
      { sampleRate: 48000, echoCancellation: true },
      { sampleRate: 44100, echoCancellation: true },
    );
    expect(comparison.sampleRate).toEqual({
      requested: 48000,
      applied: 44100,
      match: false,
    });
  });

  it("normalizes exact/ideal constraint objects", () => {
    const comparison = compareCaptureSettings(
      { channelCount: { ideal: 1 }, echoCancellation: { exact: true } },
      { channelCount: 1, echoCancellation: true },
    );
    expect(comparison.channelCount?.match).toBe(true);
    expect(comparison.echoCancellation?.match).toBe(true);
  });

  it("marks match null when applied key is missing", () => {
    const comparison = compareCaptureSettings(
      { echoCancellation: true },
      {},
    );
    expect(comparison.echoCancellation).toEqual({
      requested: true,
      applied: undefined,
      match: null,
    });
  });
});

describe("isCaptureDiagEnabled", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("is false by default in node without flags", () => {
    expect(isCaptureDiagEnabled()).toBe(false);
  });

  it("reads localStorage gelabberDiag=1", () => {
    vi.stubGlobal("localStorage", {
      getItem: (k: string) => (k === "gelabberDiag" ? "1" : null),
    });
    expect(isCaptureDiagEnabled()).toBe(true);
  });
});

describe("logCaptureSettings", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("builds comparison from track.getSettings and stashes when window exists", () => {
    const info = vi.spyOn(console, "info").mockImplementation(() => {});
    const track = {
      id: "t1",
      label: "Fake Mic",
      kind: "audio",
      readyState: "live",
      contentHint: "speech",
      getSettings: () => ({
        echoCancellation: true,
        noiseSuppression: true,
        autoGainControl: false,
        channelCount: 1,
        sampleRate: 48000,
      }),
    } as unknown as MediaStreamTrack;

    const win = {
      __gelabberCaptureDiag: undefined as unknown,
    };
    vi.stubGlobal("window", win);
    vi.stubGlobal("localStorage", { getItem: () => null });

    const requested: MediaTrackConstraints = {
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
    };
    const diag = logCaptureSettings(track, requested);
    expect(diag.comparison.autoGainControl?.match).toBe(false);
    expect(diag.comparison.echoCancellation?.match).toBe(true);
    expect(diag.playtestSaveHint).toContain("output/issue-53");
    expect(win.__gelabberCaptureDiag).toBe(diag);
    expect(info).toHaveBeenCalled();
  });
});
