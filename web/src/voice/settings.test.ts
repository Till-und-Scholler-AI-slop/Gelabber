import { afterEach, describe, expect, it } from "vitest";

import {
  AUDIO_QUALITY,
  asQuality,
  clampGain,
  clampVolume,
  micConstraints,
  resetMediaSettingsForTests,
  useMediaSettings,
} from "./settings.ts";

describe("media settings", () => {
  afterEach(() => {
    resetMediaSettingsForTests();
  });

  it("defaults to AEC/NS/AGC, Normal bitrate, full volume, toasts on", () => {
    const state = useMediaSettings.getState();
    expect(state.echoCancellation).toBe(true);
    expect(state.noiseSuppression).toBe(true);
    expect(state.autoGainControl).toBe(true);
    expect(state.quality).toBe("normal");
    expect(AUDIO_QUALITY.normal.bitrate).toBe(64_000);
    expect(AUDIO_QUALITY.phone.bitrate).toBe(24_000);
    expect(AUDIO_QUALITY.high.bitrate).toBe(128_000);
    expect(state.outputVolume).toBe(1);
    expect(state.inputGain).toBe(1);
    expect(state.messageToasts).toBe(true);
    expect(state.desktopNotify).toBe(false);
  });

  it("clamps volume and gain and rejects unknown quality", () => {
    expect(clampVolume(-1)).toBe(0);
    expect(clampVolume(2)).toBe(1);
    expect(clampGain(8)).toBe(2);
    expect(asQuality("nope")).toBe("normal");
    useMediaSettings
      .getState()
      .patch({ outputVolume: 4, inputGain: -2, quality: "high" });
    expect(useMediaSettings.getState().outputVolume).toBe(1);
    expect(useMediaSettings.getState().inputGain).toBe(0);
    expect(useMediaSettings.getState().quality).toBe("high");
  });

  it("builds mic constraints from the store, including a device hint", () => {
    expect(micConstraints()).toEqual({
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
    });
    useMediaSettings.getState().patch({
      echoCancellation: false,
      audioInputId: "mic-2",
    });
    expect(micConstraints()).toEqual({
      echoCancellation: false,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
      deviceId: { ideal: "mic-2" },
    });
  });
});
