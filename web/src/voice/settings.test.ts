import { afterEach, describe, expect, it } from "vitest";
import {
  DEFAULT_MEDIA_SETTINGS,
  AUDIO_QUALITY,
  VIDEO_RESOLUTIONS,
  VIDEO_FRAME_RATES,
  explicitStreamProfile,
  clampVideoUploadLimit,
  allocateVideoBitrates,
  audioBitrate,
  sourceAudioBitrate,
  cameraConstraints,
  micConstraints,
  resetMediaSettingsForTests,
  useMediaSettings,
  videoSendBudget,
  videoConstraintLadder,
  clampGain,
  clampVolume,
  displayConstraints,
  asStreamProfile,
  videoConstraintsFor,
  isOverconstrainedError,
} from "./settings.ts";
afterEach(resetMediaSettingsForTests);
describe("capture and explicit bandwidth preferences", () => {
  it("defaults to enhanced speech with no application bitrate cap", () => {
    expect(useMediaSettings.getState().processingMode).toBe("enhanced");
    expect(audioBitrate()).toBeNull();
    expect(sourceAudioBitrate()).toBeNull();
    expect(videoSendBudget(useMediaSettings.getState())).toBeNull();
    expect(allocateVideoBitrates(["2160p60", "balanced", "economy"])).toEqual([
      null,
      null,
      null,
    ]);
  });
  it("allows user-selected economy and a custom pool above the old 100 Mbit ceiling", () => {
    useMediaSettings.getState().patch({ economyMode: true });
    expect(audioBitrate()).toBe(AUDIO_QUALITY.normal.bitrate);
    expect(sourceAudioBitrate()).toBe(128000);
    expect(allocateVideoBitrates(["balanced", "balanced"], 0, true)).toEqual([
      1250000, 1250000,
    ]);
    expect(allocateVideoBitrates(["2160p60", "2160p60"], 500000000)).toEqual([
      250000000, 250000000,
    ]);
    expect(clampVideoUploadLimit(Infinity)).toBe(0);
    expect(clampVideoUploadLimit(-1)).toBe(0);
    useMediaSettings.getState().patch({ economyMode: false });
    expect(audioBitrate()).toBeNull();
  });
  it("keeps every capture resolution/FPS independent of bandwidth", () => {
    for (const height of VIDEO_RESOLUTIONS)
      for (const fps of VIDEO_FRAME_RATES) {
        useMediaSettings
          .getState()
          .patch({ cameraProfile: explicitStreamProfile(height, fps) });
        expect(cameraConstraints()).toMatchObject({
          height: { ideal: height, max: height },
          frameRate: { ideal: fps, max: fps },
        });
        expect(audioBitrate()).toBeNull();
        expect(videoSendBudget(useMediaSettings.getState())).toBeNull();
      }
    expect(videoConstraintLadder("camera", "2160p60")).toHaveLength(4);
  });
  it("avoids double noise filtering and offers original stereo with optional AEC", () => {
    expect(micConstraints()).toMatchObject({
      noiseSuppression: false,
      autoGainControl: false,
      channelCount: 1,
    });
    useMediaSettings
      .getState()
      .patch({ processingMode: "browser", audioInputId: "mic-2" });
    expect(micConstraints()).toMatchObject({
      noiseSuppression: true,
      autoGainControl: true,
      deviceId: { ideal: "mic-2" },
    });
    useMediaSettings
      .getState()
      .patch({ processingMode: "original", echoCancellation: false });
    expect(micConstraints()).toMatchObject({
      noiseSuppression: false,
      autoGainControl: false,
      echoCancellation: false,
      channelCount: { ideal: 2 },
      sampleRate: { ideal: 48000 },
    });
  });
  it("migrates old automatic caps without changing their browser filters or capture preferences", async () => {
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: {
        quality: "high",
        noiseSuppression: true,
        autoGainControl: true,
        cameraProfile: "2160p60",
        screenProfile: "detail",
        videoUploadLimit: 0,
      } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      processingMode: "browser",
      economyMode: false,
      cameraProfile: "2160p60",
      screenProfile: "detail",
    });
    expect(audioBitrate()).toBeNull();
    expect(videoSendBudget(useMediaSettings.getState())).toBeNull();
  });
  it("retains explicit legacy phone/custom limits and unprocessed preferences", async () => {
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: {
        quality: "phone",
        noiseSuppression: false,
        autoGainControl: false,
        videoUploadLimit: 75000000,
      } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      processingMode: "original",
      economyMode: true,
      videoUploadLimit: 75000000,
    });
    expect(audioBitrate()).toBe(24000);
  });
  it("persists the new modes and independent playback/call-sound choices", async () => {
    useMediaSettings.getState().patch({
      processingMode: "original",
      economyMode: false,
      shareSourceAudio: true,
      sourceAudioVolume: 0.35,
      callSounds: false,
      callSoundVolume: 2,
    });
    const storage = useMediaSettings.persist.getOptions().storage!;
    const saved = await storage.getItem("gelabber.media");
    resetMediaSettingsForTests();
    await storage.setItem("gelabber.media", saved!);
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      processingMode: "original",
      economyMode: false,
      shareSourceAudio: true,
      sourceAudioVolume: 0.35,
      callSounds: false,
      callSoundVolume: 1,
    });
    expect(DEFAULT_MEDIA_SETTINGS.outputVolume).toBe(1);
    expect(clampGain(5)).toBe(2);
    expect(clampVolume(-1)).toBe(0);
  });
  it("preserves camera/display fallbacks, device hints and invalid saved profile recovery", async () => {
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
    expect(videoConstraintLadder("screen", "detail")[1]).toEqual(
      videoConstraintsFor("screen", "balanced"),
    );
    expect(videoConstraintLadder("screen", "balanced")).toHaveLength(1);
    expect(asStreamProfile("__proto__")).toBe("balanced");
    expect(asStreamProfile("2160p120")).toBe("balanced");
    const rejected = new Error("unsupported");
    rejected.name = "OverconstrainedError";
    expect(isOverconstrainedError(rejected)).toBe(true);
    expect(isOverconstrainedError(new Error("denied"))).toBe(false);
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: { cameraProfile: "broken", screenProfile: "detail" } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().cameraProfile).toBe("balanced");
    expect(useMediaSettings.getState().screenProfile).toBe("detail");
  });
  it("keeps source audio and conversation playback independent across reloads", async () => {
    useMediaSettings
      .getState()
      .patch({
        shareSourceAudio: true,
        sourceAudioVolume: 0.35,
        sourceAudioMuted: true,
        outputVolume: 0.8,
      });
    const storage = useMediaSettings.persist.getOptions().storage!,
      saved = await storage.getItem("gelabber.media");
    resetMediaSettingsForTests();
    await storage.setItem("gelabber.media", saved!);
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      shareSourceAudio: true,
      sourceAudioVolume: 0.35,
      sourceAudioMuted: true,
      outputVolume: 0.8,
    });
    useMediaSettings.getState().patch({ sourceAudioVolume: -5 });
    expect(useMediaSettings.getState().sourceAudioVolume).toBe(0);
    expect(useMediaSettings.getState().outputVolume).toBe(0.8);
  });
});
