import { afterEach, describe, expect, it, vi } from "vitest";
import { CUE_LEVELS, renderCue } from "./callSoundSynth.ts";
import {
  CallSoundPlayer,
  callSoundSource,
  callSoundWave,
  playLiveSound,
} from "./callSounds.ts";
import { setNativeBridgeForTests } from "./native/bridge.ts";
import {
  DEFAULT_MEDIA_SETTINGS,
  resetMediaSettingsForTests,
  useMediaSettings,
} from "./settings.ts";

function harness() {
  const settings = { ...DEFAULT_MEDIA_SETTINGS };
  const audio = {
    src: "",
    volume: 1,
    play: vi.fn(async () => {}),
    pause: vi.fn(),
  };
  return {
    settings,
    audio,
    player: new CallSoundPlayer(
      () => settings,
      () => audio,
    ),
  };
}

afterEach(() => {
  setNativeBridgeForTests(undefined);
  vi.unstubAllGlobals();
  resetMediaSettingsForTests();
});

describe("call sounds", () => {
  it("turns each cue into 16-bit PCM at its level", () => {
    for (const [kind, rate] of [
      ["joinSelf", 24000],
      ["leaveOther", 24000],
      ["cameraOn", 48000],
    ] as const) {
      const wave = callSoundWave(kind);
      const data = new DataView(wave.buffer);
      expect(new TextDecoder().decode(wave.slice(0, 4))).toBe("RIFF");
      expect(data.getUint32(24, true)).toBe(rate);
      expect(data.getUint32(40, true)).toBe(renderCue(kind, rate).length * 2);
      let peak = 0;
      for (let i = 44; i < wave.length; i += 2)
        peak = Math.max(peak, Math.abs(data.getInt16(i, true)));
      expect(Math.abs(peak - CUE_LEVELS[kind] * 32767)).toBeLessThanOrEqual(1);
      expect(data.getInt16(wave.length - 2, true)).toBe(0);
    }
    expect(callSoundWave("joinSelf")).not.toEqual(callSoundWave("leaveSelf"));
    // The unlock cue: 120 ms of nothing.
    const silent = callSoundWave("silent");
    expect(silent.length).toBe(44 + 2880 * 2);
    expect(silent.slice(44).every((byte) => byte === 0)).toBe(true);
  });

  it("encodes a cue longer than one call's arguments as a data URI", () => {
    const wave = callSoundWave("liveStart");
    expect(wave.length).toBeGreaterThan(65536);
    const source = callSoundSource(wave);
    expect(source).toMatch(/^data:audio\/wav;base64,/);
    const decoded = Uint8Array.from(atob(source.split(",")[1]!), (char) =>
      char.charCodeAt(0),
    );
    expect(decoded).toEqual(wave);
  });

  it("honors disable, both volumes and deafen while allowing explicit control feedback", () => {
    const { settings, audio, player } = harness();
    settings.outputVolume = 0.5;
    player.play("joinSelf");
    expect(audio.volume).toBe(0.175);
    // Playback above 100 % boosts calls, not the cues.
    settings.callSoundVolume = 0.8;
    settings.outputVolume = 2;
    player.play("joinOther");
    expect(audio.volume).toBe(0.8);
    settings.callSoundVolume = DEFAULT_MEDIA_SETTINGS.callSoundVolume;
    settings.outputVolume = 0.5;
    player.setDeafened(true);
    // Arriving, leaving and what the others do stay out of a deafened call.
    for (const kind of [
      "joinSelf",
      "joinOther",
      "leaveSelf",
      "leaveOther",
      "muteOther",
      "unmuteOther",
      "viewerJoin",
      "viewerLeave",
    ] as const)
      player.play(kind);
    player.play("liveStart", { remote: true });
    player.play("liveEnd", { remote: true });
    expect(audio.play).toHaveBeenCalledTimes(2);
    // Your own controls do not.
    for (const kind of [
      "deafen",
      "muteSelf",
      "cameraOn",
      "cameraOff",
      "liveStart",
      "liveEnd",
    ] as const)
      player.play(kind);
    expect(audio.play).toHaveBeenCalledTimes(8);
    settings.callSounds = false;
    player.play("unmuteSelf");
    player.unlock();
    settings.callSounds = true;
    settings.callSoundVolume = 0;
    player.play("undeafen");
    settings.callSoundVolume = 1;
    settings.outputVolume = 0;
    player.play("undeafen");
    expect(audio.play).toHaveBeenCalledTimes(8);
  });

  it("has a switch of its own for the others' mute", () => {
    const { settings, audio, player } = harness();
    settings.otherMuteSounds = false;
    player.play("muteOther");
    player.play("unmuteOther");
    expect(audio.play).not.toHaveBeenCalled();
    player.play("muteSelf");
    player.play("joinOther");
    expect(audio.play).toHaveBeenCalledTimes(2);
    settings.otherMuteSounds = true;
    player.play("muteOther");
    expect(audio.play).toHaveBeenCalledTimes(3);
  });

  it("does not cut a cue off for one that does not play", () => {
    const { settings, audio, player } = harness();
    player.play("joinSelf");
    player.setDeafened(true);
    player.play("deafen");
    expect(audio.pause).toHaveBeenCalledTimes(2);
    player.play("joinOther");
    settings.otherMuteSounds = false;
    player.play("muteOther");
    expect(audio.pause).toHaveBeenCalledTimes(2);
    expect(audio.play).toHaveBeenCalledTimes(2);
  });

  it("gives the source up once a cue ends or is stopped", () => {
    const { audio, player } = harness();
    const element = Object.assign(audio, {
      onended: null as ((event: Event) => unknown) | null,
      removeAttribute: vi.fn((name: string) => {
        if (name === "src") element.src = "";
      }),
      load: vi.fn(),
    });
    player.play("leaveOther");
    expect(element.src).toMatch(/^data:audio\/wav/);
    element.onended?.(new Event("ended"));
    expect(element.src).toBe("");
    expect(element.load).toHaveBeenCalledTimes(1);
    player.play("joinSelf");
    player.stop();
    expect(element.pause).toHaveBeenCalled();
    expect(element.src).toBe("");
    expect(element.load).toHaveBeenCalledTimes(2);
  });

  it("cancels pending sounds and serializes device switches without late playback", async () => {
    const { settings, audio } = harness();
    let finish!: () => void;
    const routed = {
      ...audio,
      setSinkId: vi
        .fn()
        .mockImplementationOnce(
          () =>
            new Promise<void>((resolve) => {
              finish = resolve;
            }),
        )
        .mockResolvedValue(undefined),
    };
    const player = new CallSoundPlayer(
      () => settings,
      () => routed,
    );
    settings.audioOutputId = "headphones";
    player.play("joinSelf");
    await Promise.resolve();
    settings.audioOutputId = "";
    player.play("leaveOther");
    finish();
    await vi.waitFor(() => expect(audio.play).toHaveBeenCalledTimes(1));
    expect(routed.setSinkId.mock.calls).toEqual([["headphones"], [""]]);
    player.play("joinSelf");
    player.stop();
    await Promise.resolve();
    expect(audio.play).toHaveBeenCalledTimes(1);
  });

  it("still plays when the chosen speaker is refused or gone", async () => {
    const { settings, audio } = harness();
    const routed = {
      ...audio,
      setSinkId: vi
        .fn()
        .mockRejectedValueOnce(
          new DOMException("No permission to use requested device"),
        )
        .mockImplementationOnce(() => {
          throw new TypeError("not a device");
        })
        .mockResolvedValue(undefined),
    };
    const player = new CallSoundPlayer(
      () => settings,
      () => routed,
    );
    settings.audioOutputId = "unplugged-headset";
    player.play("joinSelf");
    await vi.waitFor(() => expect(audio.play).toHaveBeenCalledTimes(1));
    expect(routed.src).toContain("data:audio/wav");
    // Neither failure blocks the cues after it.
    player.play("leaveOther");
    await vi.waitFor(() => expect(audio.play).toHaveBeenCalledTimes(2));
    player.play("muteSelf");
    await vi.waitFor(() => expect(audio.play).toHaveBeenCalledTimes(3));
    expect(routed.setSinkId).toHaveBeenCalledTimes(3);
  });

  it("does not hand the desktop app's device ids to the webview", () => {
    const { settings, audio } = harness();
    const routed = {
      ...audio,
      // What Chromium answers to an id it never issued.
      setSinkId: vi.fn().mockRejectedValue(new DOMException("SecurityError")),
    };
    const player = new CallSoundPlayer(
      () => settings,
      () => routed,
    );
    setNativeBridgeForTests({
      invoke: async () => null as never,
      channel: async () => null,
    });
    settings.audioOutputId =
      "{0.0.0.00000000}.{6f1f0e4a-9a36-4f0c-8f0e-1c2d3e4f5a6b}";
    player.play("joinSelf");
    expect(audio.play).toHaveBeenCalledTimes(1);
    expect(routed.setSinkId).not.toHaveBeenCalled();
  });

  it("unlocks silently and tolerates blocked autoplay or missing browser APIs", async () => {
    const { settings, audio, player } = harness();
    audio.play.mockRejectedValue(new Error("NotAllowedError"));
    player.unlock();
    expect(audio.volume).toBe(0);
    player.play("joinSelf");
    await Promise.resolve();
    const unsupported = new CallSoundPlayer(
      () => settings,
      () => null,
    );
    expect(() => {
      unsupported.unlock();
      unsupported.play("joinSelf");
      unsupported.stop();
    }).not.toThrow();
  });

  it("sounds a Live with its own cue under the message cue's switch, not the call sounds' one", () => {
    const played: string[] = [];
    vi.stubGlobal(
      "Audio",
      class {
        src = "";
        volume = 1;
        play = vi.fn(async () => {
          played.push(this.src);
        });
        pause = vi.fn();
      },
    );
    useMediaSettings.getState().patch({ messageSound: false });
    playLiveSound();
    expect(played).toHaveLength(0);
    useMediaSettings
      .getState()
      .patch({ messageSound: true, callSounds: false });
    playLiveSound();
    expect(played).toEqual([callSoundSource(callSoundWave("liveStart"))]);
  });
});
