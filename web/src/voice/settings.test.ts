import { afterEach, describe, expect, it } from "vitest";

import {
  AUDIO_QUALITY,
  STREAM_PROFILES,
  VIDEO_SEND_BUDGET,
  VIDEO_SEND_CEILING,
  allocateVideoBitrates,
  asQuality,
  asStreamProfile,
  audioBitrate,
  cameraConstraints,
  clampGain,
  clampVolume,
  displayConstraints,
  formatVideoBitrate,
  isOverconstrainedError,
  micConstraints,
  resetMediaSettingsForTests,
  streamEstimate,
  useMediaSettings,
  videoConstraintLadder,
  videoConstraintsFor,
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
    expect(state.cameraProfile).toBe("balanced");
    expect(state.screenProfile).toBe("balanced");
    expect(state.cameraProfileApply).toBe("idle");
    expect(state.screenProfileApply).toBe("idle");
    expect(VIDEO_SEND_BUDGET).toBe(2_500_000);
    expect(VIDEO_SEND_CEILING).toBe(4_000_000);
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

  it("keeps the previous safe capture targets on the balanced default", () => {
    expect(cameraConstraints()).toEqual({
      width: { ideal: 1280, max: 1920 },
      height: { ideal: 720, max: 1080 },
      frameRate: { ideal: 30, max: 30 },
    });
    expect(displayConstraints()).toEqual({
      width: { max: 1920 },
      height: { max: 1080 },
      frameRate: { ideal: 15, max: 30 },
    });
    expect(streamEstimate("camera", "balanced")).toEqual({
      resolution: "720p ideal, max. 1080p",
      fps: "30 FPS",
      maxBitrate: 2_500_000,
    });
    expect(streamEstimate("screen", "balanced")).toEqual({
      resolution: "max. 1080p",
      fps: "15 FPS ideal, max. 30",
      maxBitrate: 2_500_000,
    });
  });

  it("builds economy and detail constraints and shows their send budget", () => {
    useMediaSettings.getState().patch({
      cameraProfile: "economy",
      screenProfile: "detail",
      videoInputId: "cam-1",
      quality: "high",
    });
    expect(cameraConstraints()).toEqual({
      width: { ideal: 854, max: 854 },
      height: { ideal: 480, max: 480 },
      frameRate: { ideal: 15, max: 15 },
      deviceId: { ideal: "cam-1" },
    });
    expect(displayConstraints()).toEqual(
      videoConstraintsFor("screen", "detail"),
    );
    expect(displayConstraints()).toEqual({
      width: { ideal: 1920, max: 1920 },
      height: { ideal: 1080, max: 1080 },
      frameRate: { ideal: 30, max: 30 },
    });
    expect(formatVideoBitrate(STREAM_PROFILES.economy.maxBitrate)).toBe(
      "0,8 Mbit/s",
    );
    expect(formatVideoBitrate(STREAM_PROFILES.detail.maxBitrate)).toBe(
      "4 Mbit/s",
    );
    expect(audioBitrate()).toBe(AUDIO_QUALITY.high.bitrate);
    expect(asStreamProfile("nope")).toBe("balanced");
    useMediaSettings.getState().patch({
      cameraProfile: "nope" as "balanced",
    });
    expect(useMediaSettings.getState().cameraProfile).toBe("balanced");
    expect(useMediaSettings.getState().quality).toBe("high");
  });

  it("shares one capped video budget and steps down when constraints are rejected", () => {
    expect(allocateVideoBitrates([])).toEqual([]);
    expect(allocateVideoBitrates(["balanced"])).toEqual([2_500_000]);
    expect(allocateVideoBitrates(["balanced", "balanced"])).toEqual([
      1_250_000, 1_250_000,
    ]);
    expect(allocateVideoBitrates(["economy"])).toEqual([800_000]);
    expect(allocateVideoBitrates(["detail"])).toEqual([4_000_000]);
    expect(allocateVideoBitrates(["detail", "detail"])).toEqual([
      2_000_000, 2_000_000,
    ]);
    expect(allocateVideoBitrates(["economy", "balanced"])).toEqual([
      606_060, 1_893_939,
    ]);
    expect(
      allocateVideoBitrates(["detail", "balanced", "economy"]).reduce(
        (sum, value) => sum + value,
        0,
      ),
    ).toBeLessThanOrEqual(VIDEO_SEND_CEILING);

    expect(
      videoConstraintLadder("camera", "detail", "cam-1").map(
        (step) => step.width,
      ),
    ).toEqual([
      { ideal: 1920, max: 1920 },
      { ideal: 1280, max: 1920 },
      { ideal: 854, max: 854 },
      undefined,
    ]);
    expect(videoConstraintLadder("camera", "detail", "cam-1")[3]).toEqual({
      deviceId: { ideal: "cam-1" },
    });
    expect(videoConstraintLadder("screen", "economy")).toHaveLength(2);
    expect(videoConstraintLadder("screen", "balanced")).toHaveLength(1);
    expect(videoConstraintLadder("screen", "detail")[1]).toEqual(
      videoConstraintsFor("screen", "balanced"),
    );
    const rejected = new Error("no");
    rejected.name = "OverconstrainedError";
    expect(isOverconstrainedError(rejected)).toBe(true);
    expect(isOverconstrainedError(new Error("denied"))).toBe(false);
  });

  it("persists stream profiles across rehydrate and drops unknown values", async () => {
    useMediaSettings.getState().patch({
      cameraProfile: "economy",
      screenProfile: "detail",
      quality: "phone",
    });
    const storage = useMediaSettings.persist.getOptions().storage;
    expect(storage).toBeTruthy();
    const saved = await storage!.getItem("gelabber.media");
    expect(saved?.state).toMatchObject({
      cameraProfile: "economy",
      screenProfile: "detail",
      quality: "phone",
    });
    useMediaSettings.setState({
      cameraProfile: "balanced",
      screenProfile: "balanced",
      quality: "normal",
      cameraProfileApply: "live",
    });
    await storage!.setItem("gelabber.media", saved!);
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().cameraProfile).toBe("economy");
    expect(useMediaSettings.getState().screenProfile).toBe("detail");
    expect(useMediaSettings.getState().quality).toBe("phone");
    expect(useMediaSettings.getState().cameraProfileApply).toBe("live");

    const broken = structuredClone(saved!) as {
      state: { cameraProfile: string; screenProfile: string };
      version?: number;
    };
    broken.state.cameraProfile = "nope";
    broken.state.screenProfile = "detail";
    await storage!.setItem("gelabber.media", broken);
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().cameraProfile).toBe("balanced");
    expect(useMediaSettings.getState().screenProfile).toBe("detail");
  });
});
