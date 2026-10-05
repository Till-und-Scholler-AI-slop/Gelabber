import { afterEach, expect, it, vi } from "vitest";
import { createMicrophoneComparison } from "./microphoneComparison.ts";
import { DEFAULT_MEDIA_SETTINGS } from "./settings.ts";

afterEach(() => vi.unstubAllGlobals());

it("ignores an old recorder error after another recording starts or the test closes", async () => {
  const recorders: Recorder[] = [];
  class Recorder {
    state = "inactive";
    onerror: (() => void) | null = null;
    ondataavailable = null;
    onstop = null;
    static isTypeSupported() {
      return true;
    }
    constructor() {
      recorders.push(this);
    }
    start() {
      this.state = "recording";
    }
    stop() {
      this.state = "inactive";
    }
  }
  const track = {
    readyState: "live",
    stop: vi.fn(),
    addEventListener() {},
    removeEventListener() {},
    getSettings: () => ({ sampleRate: 48_000, channelCount: 1 }),
  };
  const raw = { getAudioTracks: () => [track], getTracks: () => [track] };
  vi.stubGlobal("navigator", {
    mediaDevices: { getUserMedia: async () => raw },
  });
  vi.stubGlobal("MediaRecorder", Recorder);
  vi.stubGlobal("requestAnimationFrame", () => 1);
  vi.stubGlobal("cancelAnimationFrame", vi.fn());
  vi.stubGlobal(
    "AudioContext",
    class {
      state = "running";
      resume = async () => {};
      close = async () => {};
      createMediaStreamSource() {
        return { connect() {}, disconnect() {} };
      }
      createAnalyser() {
        return { fftSize: 1024, disconnect() {} };
      }
    },
  );
  const callbacks = {
    state: vi.fn(),
    levels: vi.fn(),
    clips: vi.fn(),
    recording: vi.fn(),
  };
  const comparison = createMicrophoneComparison(
    { ...DEFAULT_MEDIA_SETTINGS, processingMode: "browser" },
    callbacks,
  );
  await comparison.start();
  comparison.record();
  const old = recorders[0]!;
  comparison.endRecording();
  comparison.record();
  const clipCalls = callbacks.clips.mock.calls.length;
  old.onerror?.();
  expect(recorders.slice(2).map((recorder) => recorder.state)).toEqual([
    "recording",
    "recording",
  ]);
  expect(callbacks.recording.mock.lastCall).toEqual([true]);
  expect(callbacks.clips).toHaveBeenCalledTimes(clipCalls);
  comparison.stop();
  const stateCalls = callbacks.state.mock.calls.length;
  old.onerror?.();
  expect(callbacks.state).toHaveBeenCalledTimes(stateCalls);
});
