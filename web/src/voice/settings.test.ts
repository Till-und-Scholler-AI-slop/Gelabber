import { afterEach, describe, expect, it } from "vitest";
import { setNativeBridgeForTests, type NativeBridge } from "./native/bridge.ts";
import { loadNativeFeatures } from "./native/features.ts";
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
  sharesSourceAudio,
  sourceAudioCarriesCall,
  sourceAudioChoice,
  useMediaSettings,
  videoSendBudget,
  videoConstraintLadder,
  clampGain,
  clampVolume,
  displayConstraints,
  asStreamProfile,
  videoConstraintsFor,
  isOverconstrainedError,
  type SourceAudioShare,
} from "./settings.ts";
afterEach(resetMediaSettingsForTests);

/** The web client of v0.5.2 on the same storage, as after a rollback of the
 * server. It reads `shareSourceAudio === true` and knows nothing of the newer
 * choice; its next save (any slider) writes the keys it knows and drops the
 * rest. `pressed` is its own "Ton teilen" being set. Answers what it shares. */
async function v05Client(pressed?: boolean): Promise<boolean> {
  const storage = useMediaSettings.persist.getOptions().storage!;
  const saved = await storage.getItem("gelabber.media");
  const known = { ...saved?.state } as Record<string, unknown>;
  delete known.sourceAudioShare;
  const shareSourceAudio = pressed ?? known.shareSourceAudio === true;
  await storage.setItem("gelabber.media", {
    state: { ...known, shareSourceAudio } as never,
    version: 0,
  });
  return shareSourceAudio;
}

/** This client again, loading what is stored now. */
async function v06Client(): Promise<SourceAudioShare> {
  await useMediaSettings.persist.rehydrate();
  return useMediaSettings.getState().sourceAudioShare;
}
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
      sourceAudioShare: "on",
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
      sourceAudioShare: "on",
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
    useMediaSettings.getState().patch({
      sourceAudioShare: "on",
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
      sourceAudioShare: "on",
      sourceAudioVolume: 0.35,
      sourceAudioMuted: true,
      outputVolume: 0.8,
    });
    useMediaSettings.getState().patch({ sourceAudioVolume: -5 });
    expect(useMediaSettings.getState().sourceAudioVolume).toBe(0);
    expect(useMediaSettings.getState().outputVolume).toBe(0.8);
  });
});

describe("stream sound: the stored choice", () => {
  const storage = () => useMediaSettings.persist.getOptions().storage!;
  async function reloadWith(state: Record<string, unknown>) {
    resetMediaSettingsForTests();
    await storage().setItem("gelabber.media", { state: state as never });
    await useMediaSettings.persist.rehydrate();
    return useMediaSettings.getState().sourceAudioShare;
  }

  it("has made no choice by default", () => {
    expect(DEFAULT_MEDIA_SETTINGS.sourceAudioShare).toBe("auto");
    expect(useMediaSettings.getState().sourceAudioShare).toBe("auto");
  });

  it("reads the old boolean: false was nobody's choice, true was", async () => {
    // Up to v0.5 the default was stored as false like a deliberate "off".
    expect(await reloadWith({ shareSourceAudio: false })).toBe("auto");
    expect(await reloadWith({ shareSourceAudio: true })).toBe("on");
    expect(await reloadWith({ quality: "high" })).toBe("auto");
    // The old key is storage only: it does not linger in the state.
    expect(useMediaSettings.getState()).not.toHaveProperty("shareSourceAudio");
  });

  it("keeps an explicit off apart from the old default across reloads", async () => {
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    const saved = (await storage().getItem("gelabber.media"))!;
    // Next to it the boolean an older client reads: not switched on.
    expect(saved.state).toMatchObject({
      sourceAudioShare: "off",
      shareSourceAudio: false,
    });
    resetMediaSettingsForTests();
    await storage().setItem("gelabber.media", saved);
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().sourceAudioShare).toBe("off");
    // The new choice wins over a leftover of the old boolean.
    expect(
      await reloadWith({ sourceAudioShare: "off", shareSourceAudio: true }),
    ).toBe("off");
    expect(
      await reloadWith({ sourceAudioShare: "on", shareSourceAudio: false }),
    ).toBe("on");
  });

  it("takes anything else for no choice", async () => {
    expect(await reloadWith({ sourceAudioShare: true })).toBe("auto");
    expect(await reloadWith({ sourceAudioShare: "yes" })).toBe("auto");
    useMediaSettings.getState().patch({ sourceAudioShare: "loud" as never });
    expect(useMediaSettings.getState().sourceAudioShare).toBe("auto");
  });
});

describe("stream sound: a rollback to v0.5 and back", () => {
  const choose = (sourceAudioShare: SourceAudioShare) =>
    useMediaSettings.getState().patch({ sourceAudioShare });
  const stored = async () =>
    (await useMediaSettings.persist
      .getOptions()
      .storage!.getItem("gelabber.media"))!.state as Record<string, unknown>;

  it("leaves the old client its boolean: switched on stays on there", async () => {
    choose("on");
    expect(await stored()).toMatchObject({
      sourceAudioShare: "on",
      shareSourceAudio: true,
    });
    expect(await v05Client()).toBe(true);
    // Its save has dropped the newer key; the boolean still says on.
    expect(await stored()).not.toHaveProperty("sourceAudioShare");
    expect(await v06Client()).toBe("on");

    // Neither an off nor no choice is anything it would share.
    choose("off");
    expect(await v05Client()).toBe(false);
    choose("auto");
    expect(await v05Client()).toBe(false);
  });

  it("finds an off again that the old client could not keep", async () => {
    choose("off");
    expect(await v05Client()).toBe(false);
    expect(await stored()).not.toHaveProperty("sourceAudioShare");
    expect(await v06Client()).toBe("off");
    // And saves it as before.
    useMediaSettings.getState().patch({ outputVolume: 0.5 });
    expect(await stored()).toMatchObject({
      sourceAudioShare: "off",
      shareSourceAudio: false,
    });
  });

  it("takes what was switched in the old client over what it had stored", async () => {
    // Off here, switched on there.
    choose("off");
    expect(await v05Client(true)).toBe(true);
    expect(await v06Client()).toBe("on");
    // On here, switched off there: the old client read the on, so its false
    // is the user's off.
    choose("on");
    expect(await v05Client(false)).toBe(false);
    expect(await v06Client()).toBe("off");
    // On here and left alone there stays on.
    choose("on");
    expect(await v05Client()).toBe(true);
    expect(await v06Client()).toBe("on");
    // On, off again there, both in the old client, after an off here.
    choose("off");
    await v05Client(true);
    await v05Client(false);
    expect(await v06Client()).toBe("off");
  });

  it("has nothing to restore for a client that never chose", async () => {
    expect(await v05Client()).toBe(false);
    expect(await v06Client()).toBe("auto");
    expect(await v05Client(true)).toBe(true);
    expect(await v06Client()).toBe("on");
  });
});

describe("stream sound: what a share does", () => {
  afterEach(() => setNativeBridgeForTests(undefined));

  /** The desktop app, answering `media_info` with `info`. */
  async function desktopApp(info: unknown) {
    setNativeBridgeForTests({
      invoke: async (command: string) =>
        command === "media_info" ? info : Promise.reject(new Error(command)),
      channel: async () => null,
    } as unknown as NativeBridge);
    await loadNativeFeatures();
  }
  const choices = () => ({
    auto: sharesSourceAudio({ sourceAudioShare: "auto" }),
    on: sharesSourceAudio({ sourceAudioShare: "on" }),
    off: sharesSourceAudio({ sourceAudioShare: "off" }),
  });
  const carriesCall = (sourceAudioShare: "auto" | "on", sourceAudioApp = "") =>
    sourceAudioCarriesCall({ sourceAudioShare, sourceAudioApp });
  /** What the switch stores when it is flipped. */
  const switched = () => ({
    on: sourceAudioChoice(true),
    off: sourceAudioChoice(false),
  });
  const APP_05 = { abi: 7, version: "0.5.2", platform: "linux" };
  const APP_06 = {
    features: ["screen", "camera", "app-audio", "app-audio-excludes-self"],
  };

  it("browser: off unless switched on, as before", () => {
    setNativeBridgeForTests(null);
    expect(choices()).toEqual({ auto: false, on: true, off: false });
    expect(sharesSourceAudio()).toBe(false);
    expect(carriesCall("on")).toBe(false);
    // Off is what a browser does anyway: nothing to remember.
    expect(switched()).toEqual({ on: "on", off: "auto" });
  });

  it("0.5.x app: off by default, and switched on it also carries the call", async () => {
    await desktopApp(APP_05);
    expect(choices()).toEqual({ auto: false, on: true, off: false });
    expect(carriesCall("auto")).toBe(false);
    expect(carriesCall("on")).toBe(true);
    // One application's sound is not the call's.
    expect(carriesCall("on", "firefox")).toBe(false);
    expect(switched()).toEqual({ on: "on", off: "auto" });
  });

  it("v0.6 app whose core still captures itself: like 0.5.x", async () => {
    await desktopApp({
      features: ["screen", "camera", "app-audio", "video-frames"],
    });
    expect(choices()).toEqual({ auto: false, on: true, off: false });
    expect(carriesCall("on")).toBe(true);
    expect(switched()).toEqual({ on: "on", off: "auto" });
  });

  it("v0.6 app that leaves itself out: on unless switched off", async () => {
    await desktopApp(APP_06);
    expect(choices()).toEqual({ auto: true, on: true, off: false });
    expect(sharesSourceAudio()).toBe(true);
    expect(carriesCall("auto")).toBe(false);
    expect(carriesCall("on")).toBe(false);
    // A choice from before v0.6 was none: such users get the new default.
    useMediaSettings.getState().patch({ sourceAudioShare: "off" });
    expect(sharesSourceAudio()).toBe(false);
    // Here off differs from what the app does unasked: it is kept.
    expect(switched()).toEqual({ on: "on", off: "off" });
  });

  it("an app without application sound shares none, whatever was chosen", async () => {
    await desktopApp({ features: ["camera", "video-frames"] });
    expect(choices()).toEqual({ auto: false, on: false, off: false });
    expect(carriesCall("on")).toBe(false);
    expect(switched().off).toBe("auto");
  });

  it("promises nothing before the desktop app has answered", () => {
    setNativeBridgeForTests({
      invoke: () => new Promise(() => {}),
      channel: async () => null,
    } as unknown as NativeBridge);
    expect(choices()).toEqual({ auto: false, on: false, off: false });
    expect(switched().off).toBe("auto");
  });

  it("switched on and off again in a 0.5.x app, the updated app still shares sound unasked", async () => {
    const storage = useMediaSettings.persist.getOptions().storage!;
    const flip = (on: boolean) =>
      useMediaSettings
        .getState()
        .patch({ sourceAudioShare: sourceAudioChoice(on) });
    /** The same stored settings, read by the app that runs now. */
    async function restart() {
      const saved = (await storage.getItem("gelabber.media"))!;
      resetMediaSettingsForTests();
      await storage.setItem("gelabber.media", saved);
      await useMediaSettings.persist.rehydrate();
      return saved.state as { sourceAudioShare?: unknown };
    }

    await desktopApp(APP_05);
    flip(true);
    // The warning that sends the user to the update.
    expect(sourceAudioCarriesCall()).toBe(true);
    flip(false);
    expect(sharesSourceAudio()).toBe(false);
    expect(sourceAudioCarriesCall()).toBe(false);

    await desktopApp(APP_06);
    expect((await restart()).sourceAudioShare).toBe("auto");
    expect(sharesSourceAudio()).toBe(true);

    // Switched off in the updated app is a choice, and stays one.
    flip(false);
    expect((await restart()).sourceAudioShare).toBe("off");
    expect(sharesSourceAudio()).toBe(false);
    flip(true);
    expect((await restart()).sourceAudioShare).toBe("on");
    expect(sharesSourceAudio()).toBe(true);
  });

  it("switched off in the v0.6 app, a server rolled back and updated again shares no sound", async () => {
    await desktopApp(APP_06);
    useMediaSettings
      .getState()
      .patch({ sourceAudioShare: sourceAudioChoice(false) });
    expect(sharesSourceAudio()).toBe(false);
    // The v0.5 web client in the same app saves once, and v0.6 returns.
    expect(await v05Client()).toBe(false);
    expect(await v06Client()).toBe("off");
    expect(sharesSourceAudio()).toBe(false);
    // Without that off the app shares sound unasked, as on a first upgrade.
    useMediaSettings.getState().patch({ sourceAudioShare: "auto" });
    await v05Client();
    expect(await v06Client()).toBe("auto");
    expect(sharesSourceAudio()).toBe(true);
  });
});
