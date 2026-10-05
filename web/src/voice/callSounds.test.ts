import { describe, expect, it, vi } from "vitest";
import { CallSoundPlayer, callSoundWave } from "./callSounds.ts";
import { DEFAULT_MEDIA_SETTINGS } from "./settings.ts";

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

describe("call sounds", () => {
  it("produces short, distinct, bounded PCM with silent ends", () => {
    const join = callSoundWave("join");
    expect(new TextDecoder().decode(join.slice(0, 4))).toBe("RIFF");
    expect(join).not.toEqual(callSoundWave("leave"));
    expect(join.length).toBe(44 + 24000 * 0.24 * 2);
    const data = new DataView(join.buffer);
    expect(data.getInt16(44, true)).toBe(0);
    expect(data.getInt16(join.length - 2, true)).toBe(0);
    let peak = 0;
    for (let i = 44; i < join.length; i += 2)
      peak = Math.max(peak, Math.abs(data.getInt16(i, true)));
    expect(peak).toBeGreaterThan(9000);
    expect(peak).toBeLessThan(10000);
  });

  it("honors disable, both volumes and deafen while allowing explicit control feedback", () => {
    const { settings, audio, player } = harness();
    settings.outputVolume = 0.5;
    player.play("join");
    expect(audio.volume).toBe(0.175);
    player.setDeafened(true);
    player.play("join");
    player.play("leave");
    expect(audio.play).toHaveBeenCalledTimes(1);
    player.play("deafen");
    expect(audio.play).toHaveBeenCalledTimes(2);
    settings.callSounds = false;
    player.play("unmute");
    player.unlock();
    settings.callSounds = true;
    settings.callSoundVolume = 0;
    player.play("undeafen");
    settings.callSoundVolume = 1;
    settings.outputVolume = 0;
    player.play("undeafen");
    expect(audio.play).toHaveBeenCalledTimes(2);
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
    player.play("join");
    await Promise.resolve();
    settings.audioOutputId = "";
    player.play("leave");
    finish();
    await vi.waitFor(() => expect(audio.play).toHaveBeenCalledTimes(1));
    expect(routed.setSinkId.mock.calls).toEqual([["headphones"], [""]]);
    player.play("join");
    player.stop();
    await Promise.resolve();
    expect(audio.play).toHaveBeenCalledTimes(1);
  });

  it("unlocks silently and tolerates blocked autoplay or missing browser APIs", async () => {
    const { settings, audio, player } = harness();
    audio.play.mockRejectedValue(new Error("NotAllowedError"));
    player.unlock();
    expect(audio.volume).toBe(0);
    player.play("join");
    await Promise.resolve();
    const unsupported = new CallSoundPlayer(
      () => settings,
      () => null,
    );
    expect(() => {
      unsupported.unlock();
      unsupported.play("join");
      unsupported.stop();
    }).not.toThrow();
  });
});
