import { describe, expect, it, vi } from "vitest";
import {
  prioritizeSender,
  readSenderPriority,
  type MediaPriority,
} from "./mediaPriority.ts";

function sender(network = true) {
  let parameters = {
    transactionId: "original",
    codecs: [
      { mimeType: "audio/opus", sdpFmtpLine: "maxaveragebitrate=72000" },
    ],
    encodings: [
      {
        maxBitrate: 72_000,
        maxFramerate: 30,
        priority: "low" as MediaPriority,
        ...(network ? { networkPriority: "low" as MediaPriority } : {}),
      },
    ],
  };
  return {
    getParameters: () => structuredClone(parameters),
    setParameters: vi.fn(async (next: typeof parameters) => {
      parameters = structuredClone(next);
    }),
  };
}
describe("best effort RTP priorities", () => {
  it("prioritizes audio while preserving every existing encoding and peer codec limit", async () => {
    const audio = sender();
    await prioritizeSender(audio, "audio");
    expect(audio.getParameters()).toMatchObject({
      transactionId: "original",
      codecs: [{ sdpFmtpLine: "maxaveragebitrate=72000" }],
      encodings: [
        {
          maxBitrate: 72_000,
          maxFramerate: 30,
          priority: "high",
          networkPriority: "high",
        },
      ],
    });
    expect(audio.setParameters).toHaveBeenCalledTimes(2);
    await prioritizeSender(audio, "video");
    expect(readSenderPriority(audio)).toEqual({
      priority: "low",
      networkPriority: "low",
    });
  });
  it("does not invent network support or a default bitrate cap", async () => {
    const audio = sender(false),
      native = audio.getParameters;
    audio.getParameters = () => {
      const parameters = native();
      delete (parameters.encodings[0] as { maxBitrate?: number }).maxBitrate;
      return parameters;
    };
    await prioritizeSender(audio, "audio");
    expect(audio.setParameters).toHaveBeenCalledOnce();
    expect(audio.getParameters().encodings[0]).not.toHaveProperty(
      "networkPriority",
    );
    expect(audio.getParameters().encodings[0]).not.toHaveProperty("maxBitrate");
    expect(readSenderPriority(audio)).toEqual({
      priority: "high",
      networkPriority: null,
    });
  });
  it("keeps working priority and caps when only DSCP is rejected", async () => {
    const audio = sender(),
      set = audio.setParameters;
    audio.setParameters = vi.fn(async (parameters) => {
      if (parameters.encodings[0]?.networkPriority === "high")
        throw new Error("DSCP unsupported");
      await set(parameters);
    });
    await expect(prioritizeSender(audio, "audio")).resolves.toBeUndefined();
    expect(readSenderPriority(audio)).toEqual({
      priority: "high",
      networkPriority: "low",
    });
    expect(audio.getParameters().encodings[0]?.maxBitrate).toBe(72_000);
  });
  it("leaves working parameters intact when hints are rejected and stops stale follow-up writes", async () => {
    const audio = sender(),
      previous = audio.getParameters();
    audio.setParameters = vi.fn(async () => {
      throw new Error("priority unavailable");
    });
    await expect(prioritizeSender(audio, "audio")).resolves.toBeUndefined();
    expect(audio.getParameters()).toEqual(previous);
    const guarded = sender(),
      set = guarded.setParameters;
    let current = true;
    guarded.setParameters = vi.fn(async (parameters) => {
      await set(parameters);
      current = false;
    });
    await prioritizeSender(guarded, "audio", () => current);
    expect(guarded.setParameters).toHaveBeenCalledOnce();
    expect(readSenderPriority(guarded)).toEqual({
      priority: "high",
      networkPriority: "low",
    });
  });
});
