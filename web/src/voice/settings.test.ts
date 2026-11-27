import { afterEach, describe, expect, it, vi } from "vitest";

import { useLocale } from "../i18n/locale.ts";

import { pushToTalkKeyLabel } from "./pushToTalk.ts";
import {
  DEFAULT_MEDIA_SETTINGS,
  AUDIO_QUALITY,
  VIDEO_RESOLUTIONS,
  VIDEO_FRAME_RATES,
  explicitStreamProfile,
  clampVideoUploadLimit,
  uploadLimitFromField,
  allocateVideoBitrates,
  audioBitrate,
  sourceAudioBitrate,
  cameraConstraints,
  micConstraints,
  resetMediaSettingsForTests,
  sharesSourceAudio,
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
  MAX_PLAYBACK_VOLUME,
  PLAYBACK_SLIDER_MAX,
  playbackSliderPosition,
  playbackSliderVolume,
  asProcessingMode,
  AUDIO_PROCESSING,
  GATE_THRESHOLD_DEFAULT,
  GATE_THRESHOLD_MAX,
  GATE_THRESHOLD_MIN,
  noteStreamProfileApply,
  playbackSliderFill,
  type MediaSettings,
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
  it("reads the custom upload limit field only once it holds a limit", () => {
    // Half-typed: kept as text, the stored limit and its mode stay.
    for (const text of ["", "0", "0.", "-", "abc"])
      expect(uploadLimitFromField(text)).toBeNull();
    expect(uploadLimitFromField("0.5")).toBe(500_000);
    expect(uploadLimitFromField("12")).toBe(12_000_000);
    expect(uploadLimitFromField("1,5")).toBe(1_500_000);
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
      autoGainControl: true,
      channelCount: 1,
    });
    useMediaSettings.getState().patch({ autoGainControl: false });
    expect(micConstraints()).toMatchObject({ autoGainControl: false });
    useMediaSettings.getState().patch({ autoGainControl: true });
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
  it("keeps Strong as chosen and captures for it like Standard (GEL-28)", async () => {
    useMediaSettings.getState().patch({ processingMode: "strong" });
    expect(useMediaSettings.getState().processingMode).toBe("strong");
    expect(micConstraints()).toEqual(
      micConstraints({
        ...useMediaSettings.getState(),
        processingMode: "enhanced",
      }),
    );
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().processingMode).toBe("strong");
    // Standard stays the default and the answer to anything unknown, as an
    // older client reads "strong".
    expect(asProcessingMode("strong")).toBe("strong");
    expect(asProcessingMode("stronger")).toBe("enhanced");
    expect(DEFAULT_MEDIA_SETTINGS.processingMode).toBe("enhanced");
    expect(Object.keys(AUDIO_PROCESSING)).toEqual([
      "enhanced",
      "strong",
      "browser",
      "original",
    ]);
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
      // Saved before it existed: the others' mute cues are on.
      otherMuteSounds: true,
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
  it("does not restore a corrupted entry's values or unknown keys", async () => {
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: {
        inputGain: 50,
        audioInputId: 7,
        audioOutputId: null,
        videoInputId: { id: "cam" },
        echoCancellation: "no",
        noiseSuppression: 0,
        autoGainControl: "yes",
        messageToasts: "off",
        desktopNotify: 1,
        injected: "value",
      } as never,
    });
    await useMediaSettings.persist.rehydrate();
    const state = useMediaSettings.getState();
    expect(state).toMatchObject({
      inputGain: 2,
      audioInputId: DEFAULT_MEDIA_SETTINGS.audioInputId,
      audioOutputId: DEFAULT_MEDIA_SETTINGS.audioOutputId,
      videoInputId: DEFAULT_MEDIA_SETTINGS.videoInputId,
      echoCancellation: DEFAULT_MEDIA_SETTINGS.echoCancellation,
      noiseSuppression: DEFAULT_MEDIA_SETTINGS.noiseSuppression,
      autoGainControl: DEFAULT_MEDIA_SETTINGS.autoGainControl,
      messageToasts: DEFAULT_MEDIA_SETTINGS.messageToasts,
      desktopNotify: DEFAULT_MEDIA_SETTINGS.desktopNotify,
    });
    expect(state).not.toHaveProperty("injected");
  });
  it("restores valid devices, filters and notices", async () => {
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: {
        inputGain: 1.5,
        audioInputId: "mic",
        audioOutputId: "speaker",
        videoInputId: "cam",
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
        messageToasts: false,
        desktopNotify: true,
      } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      inputGain: 1.5,
      audioInputId: "mic",
      audioOutputId: "speaker",
      videoInputId: "cam",
      echoCancellation: false,
      noiseSuppression: false,
      autoGainControl: false,
      messageToasts: false,
      desktopNotify: true,
    });
  });
  it("persists the new modes and independent playback/call-sound choices", async () => {
    useMediaSettings.getState().patch({
      processingMode: "original",
      economyMode: false,
      sourceAudioShare: "on",
      sourceAudioVolume: 0.35,
      callSounds: false,
      callSoundVolume: 2,
      otherMuteSounds: false,
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
      otherMuteSounds: false,
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
  it("starts as before and remembers keeping text sharp (GEL-26)", async () => {
    expect(useMediaSettings.getState().screenMode).toBe("smooth");
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: { screenMode: "sharp" } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().screenMode).toBe("sharp");
    // Settings from before the switch, or a broken value: as before.
    for (const state of [{}, { screenMode: "blurry" }]) {
      await storage.setItem("gelabber.media", { state: state as never });
      await useMediaSettings.persist.rehydrate();
      expect(useMediaSettings.getState().screenMode).toBe("smooth");
    }
    useMediaSettings.getState().patch({ screenMode: "fast" as never });
    expect(useMediaSettings.getState().screenMode).toBe("smooth");
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
  it("allows playback up to 400 % and clamps stored values", async () => {
    useMediaSettings
      .getState()
      .patch({ outputVolume: 1.6, sourceAudioVolume: 9 });
    expect(useMediaSettings.getState()).toMatchObject({
      outputVolume: 1.6,
      sourceAudioVolume: MAX_PLAYBACK_VOLUME,
    });
    useMediaSettings.getState().patch({ callSoundVolume: 1.6 });
    expect(useMediaSettings.getState().callSoundVolume).toBe(1);
    const storage = useMediaSettings.persist.getOptions().storage!;
    await storage.setItem("gelabber.media", {
      state: { outputVolume: 7, sourceAudioVolume: 3.5 } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      outputVolume: 4,
      sourceAudioVolume: 3.5,
    });
  });

  it("keeps 100 % in the middle of a playback slider", () => {
    expect(PLAYBACK_SLIDER_MAX).toBe(200);
    expect(playbackSliderPosition(0)).toBe(0);
    expect(playbackSliderPosition(0.5)).toBe(50);
    expect(playbackSliderPosition(1)).toBe(100);
    expect(playbackSliderPosition(2.5)).toBe(150);
    expect(playbackSliderPosition(MAX_PLAYBACK_VOLUME)).toBe(200);
    expect(playbackSliderPosition(9)).toBe(200);
    expect(playbackSliderVolume(0)).toBe(0);
    expect(playbackSliderVolume(73)).toBe(0.73);
    expect(playbackSliderVolume(100)).toBe(1);
    expect(playbackSliderVolume(150)).toBe(2.5);
    expect(playbackSliderVolume(200)).toBe(MAX_PLAYBACK_VOLUME);
    // Every step lands back on itself.
    for (let step = 0; step <= PLAYBACK_SLIDER_MAX; step++)
      expect(playbackSliderPosition(playbackSliderVolume(step))).toBe(step);
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
    expect(saved.state).toMatchObject({ sourceAudioShare: "off" });
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

describe("stream sound: the entry of v0.6 to v0.14", () => {
  it("is removed when the settings load", async () => {
    const KEY = "gelabber.media.source-audio-share";
    const stored = new Map([
      [KEY, "off"],
      ["gelabber.other", "kept"],
    ]);
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => void stored.set(key, value),
      removeItem: (key: string) => void stored.delete(key),
    });
    try {
      vi.resetModules();
      await import("./settings.ts");
      expect([...stored.keys()]).toEqual(["gelabber.other"]);
    } finally {
      vi.unstubAllGlobals();
      vi.resetModules();
    }
  });
});

describe("stream sound: what a share does", () => {
  it("is off unless switched on", () => {
    expect({
      auto: sharesSourceAudio({ sourceAudioShare: "auto" }),
      on: sharesSourceAudio({ sourceAudioShare: "on" }),
      off: sharesSourceAudio({ sourceAudioShare: "off" }),
    }).toEqual({ auto: false, on: true, off: false });
    expect(sharesSourceAudio()).toBe(false);
    // Off is what a browser does anyway: nothing to remember.
    expect({
      on: sourceAudioChoice(true),
      off: sourceAudioChoice(false),
    }).toEqual({ on: "on", off: "auto" });
  });
});

describe("push-to-talk key: the stored label", () => {
  const storage = () => useMediaSettings.persist.getOptions().storage!;
  afterEach(() => useLocale.setState({ locale: "de" }));

  it("names a key saved with a German label in the current language", async () => {
    await storage().setItem("gelabber.media", {
      state: {
        voiceInput: "ptt",
        pttKey: "Space",
        pttKeyLabel: "Leertaste",
      } as never,
    });
    await useMediaSettings.persist.rehydrate();
    const s = useMediaSettings.getState();
    expect(s.pttKeyLabel).toBe("");
    useLocale.setState({ locale: "en" });
    expect(s.pttKeyLabel || pushToTalkKeyLabel(s.pttKey)).toBe("Space");
  });

  it("keeps a printed key's character", async () => {
    await storage().setItem("gelabber.media", {
      state: { pttKey: "KeyY", pttKeyLabel: "Z" } as never,
    });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState().pttKeyLabel).toBe("Z");
  });
});

describe("every setting on its own", () => {
  const storage = () => useMediaSettings.persist.getOptions().storage!;
  const settingsOf = (state: MediaSettings): MediaSettings =>
    Object.fromEntries(
      Object.keys(DEFAULT_MEDIA_SETTINGS).map((key) => [
        key,
        state[key as keyof MediaSettings],
      ]),
    ) as MediaSettings;
  async function reload() {
    const saved = await storage().getItem("gelabber.media");
    resetMediaSettingsForTests();
    await storage().setItem("gelabber.media", saved!);
    await useMediaSettings.persist.rehydrate();
    return settingsOf(useMediaSettings.getState());
  }
  // A choice other than the default for every setting.
  const CHOSEN: MediaSettings = {
    audioInputId: "mic-2",
    audioOutputId: "headset",
    videoInputId: "cam-2",
    echoCancellation: false,
    noiseSuppression: false,
    autoGainControl: false,
    outputVolume: 1.6,
    inputGain: 1.5,
    sourceAudioShare: "on",
    sourceAudioApp: "player",
    sourceAudioVolume: 0.5,
    sourceAudioMuted: true,
    quality: "high",
    processingMode: "browser",
    economyMode: true,
    cameraProfile: "economy",
    screenProfile: "detail",
    screenMode: "sharp",
    videoUploadLimit: 5_000_000,
    callSounds: false,
    callSoundVolume: 0.8,
    otherMuteSounds: false,
    messageToasts: false,
    desktopNotify: true,
    messageSound: false,
    voiceInput: "ptt",
    pttKey: "KeyV",
    pttKeyLabel: "V",
    pttReleaseDelay: 400,
    gateAuto: false,
    gateThresholdDb: -62,
    joinMuted: true,
    joinDeafened: true,
  };

  it("changes the one setting it is given and keeps the others", () => {
    for (const key of Object.keys(CHOSEN) as (keyof MediaSettings)[])
      for (const [from, to] of [
        [DEFAULT_MEDIA_SETTINGS, CHOSEN],
        [CHOSEN, DEFAULT_MEDIA_SETTINGS],
      ]) {
        resetMediaSettingsForTests();
        useMediaSettings.getState().patch(from);
        useMediaSettings.getState().patch({ [key]: to[key] });
        expect(settingsOf(useMediaSettings.getState()), key).toEqual({
          ...from,
          [key]: to[key],
        });
      }
  });

  it("remembers every choice, and every default, across a reload", async () => {
    expect(await reload()).toEqual(DEFAULT_MEDIA_SETTINGS);
    useMediaSettings.getState().patch(CHOSEN);
    expect(await reload()).toEqual(CHOSEN);
  });

  it("keeps the sounds on, the upload unlimited and no app or key chosen for settings saved before them", async () => {
    await storage().setItem("gelabber.media", { state: {} as never });
    await useMediaSettings.persist.rehydrate();
    expect(useMediaSettings.getState()).toMatchObject({
      callSounds: true,
      otherMuteSounds: true,
      messageSound: true,
      videoUploadLimit: 0,
      sourceAudioApp: "",
      pttKey: "",
    });
  });

  it("keeps the browser's filters for a saved filter that was only partly off", async () => {
    for (const state of [
      { noiseSuppression: false, autoGainControl: true },
      { noiseSuppression: true, autoGainControl: false },
    ]) {
      await storage().setItem("gelabber.media", { state: state as never });
      await useMediaSettings.persist.rehydrate();
      expect(
        useMediaSettings.getState().processingMode,
        JSON.stringify(state),
      ).toBe("browser");
    }
  });

  it("keeps the input sensitivity inside the slider's range", () => {
    const threshold = (value: number) => {
      useMediaSettings.getState().patch({ gateThresholdDb: value });
      return useMediaSettings.getState().gateThresholdDb;
    };
    expect(threshold(-62.4)).toBe(-62);
    expect(threshold(-200)).toBe(GATE_THRESHOLD_MIN);
    expect(threshold(12)).toBe(GATE_THRESHOLD_MAX);
    expect(threshold(Number.NaN)).toBe(GATE_THRESHOLD_DEFAULT);
  });

  it("notes a profile change on the stream it was made for", () => {
    noteStreamProfileApply("screen", "next");
    expect(useMediaSettings.getState()).toMatchObject({
      cameraProfileApply: "idle",
      screenProfileApply: "next",
    });
    noteStreamProfileApply("camera", "live");
    expect(useMediaSettings.getState()).toMatchObject({
      cameraProfileApply: "live",
      screenProfileApply: "next",
    });
  });

  it("fills a playback slider up to its knob", () => {
    expect(playbackSliderFill(0)).toBe("0%");
    expect(playbackSliderFill(1)).toBe("50%");
    expect(playbackSliderFill(MAX_PLAYBACK_VOLUME)).toBe("100%");
  });
});
