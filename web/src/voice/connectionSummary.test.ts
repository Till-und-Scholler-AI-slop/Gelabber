import { describe, expect, it } from "vitest";

import {
  directionTotal,
  flowGroups,
  lossTone,
  pingHistory,
  pingTone,
} from "./connectionSummary.ts";
import type {
  ConnectionSnapshot,
  DiagnosticSample,
  FlowStats,
} from "./diagnostics.ts";

function flow(partial: Partial<FlowStats>): FlowStats {
  return {
    direction: "recv",
    kind: "audio",
    source: "voice",
    codec: "opus",
    measuredBitrateBps: null,
    configuredMaxBitrateBps: null,
    configuredMaxFps: null,
    senderPriority: null,
    senderNetworkPriority: null,
    packetLoss: null,
    packetsLost: null,
    jitterMs: null,
    rttMs: null,
    concealedSamples: null,
    totalSamplesReceived: null,
    concealedRatio: null,
    width: null,
    height: null,
    fps: null,
    qualityLimitationReason: null,
    ...partial,
  };
}

function snapshot(
  flows: FlowStats[],
  rttMs: number | null = 20,
  role: "voice" | "watch" = "voice",
): ConnectionSnapshot {
  return {
    role,
    transport: {
      path: "direct",
      rttMs,
      availableIncomingBps: null,
      availableOutgoingBps: null,
    },
    flows,
  };
}

function sample(rttMs: number | null): DiagnosticSample {
  return {
    at: "2026-10-28T12:00:00.000Z",
    phase: "voice-only",
    caps: { audioMaxBitrate: null, videoSendBudget: null, videoMaxFps: 60 },
    voice: snapshot([flow({ packetLoss: 0 })], rttMs),
    watch: null,
  };
}

describe("ping history", () => {
  it("keeps the newest samples on the right and pads the start", () => {
    expect(pingHistory([sample(10), sample(12)], 4)).toEqual([
      null,
      null,
      10,
      12,
    ]);
    const many = [5, 6, 7, 8, 9].map(sample);
    expect(pingHistory(many, 3)).toEqual([7, 8, 9]);
    // A sample without a figure leaves a gap.
    const blank = { ...sample(null), voice: null };
    expect(pingHistory([sample(10), blank], 2)).toEqual([10, null]);
  });

  it("colours a ping where the call bar drops to two and one bar", () => {
    expect(pingTone(14)).toBe("good");
    expect(pingTone(299)).toBe("good");
    expect(pingTone(300)).toBe("weak");
    expect(pingTone(500)).toBe("bad");
    expect(lossTone(0.03)).toBe("good");
    expect(lossTone(0.05)).toBe("weak");
    expect(lossTone(0.1)).toBe("bad");
  });
});

describe("flow groups", () => {
  it("puts everyone's voice in one row with the worst loss", () => {
    const groups = flowGroups(
      snapshot([
        flow({ measuredBitrateBps: 40_000, packetLoss: 0, jitterMs: 2 }),
        flow({ measuredBitrateBps: 30_000, packetLoss: 0.03, jitterMs: 9 }),
        flow({ measuredBitrateBps: null, packetLoss: null }),
        flow({ direction: "send", measuredBitrateBps: 38_000 }),
      ]),
      "voice",
    );
    expect(groups.map((group) => [group.direction, group.name])).toEqual([
      ["send", "Sprache"],
      ["recv", "Sprache"],
    ]);
    const received = groups[1]!;
    expect(received.count).toBe(3);
    expect(received.bitrateBps).toBe(70_000);
    expect(received.loss).toBe(0.03);
    expect(received.jitterMs).toBe(9);
    expect(received.codec).toBe("opus");
  });

  it("draws a camera's layers as one row with its largest picture", () => {
    const [camera] = flowGroups(
      snapshot([
        flow({
          direction: "send",
          kind: "video",
          source: "camera",
          codec: "VP8",
          measuredBitrateBps: 150_000,
          width: 320,
          height: 180,
        }),
        flow({
          direction: "send",
          kind: "video",
          source: "camera",
          codec: "VP8",
          measuredBitrateBps: 2_000_000,
          width: 1280,
          height: 720,
          fps: 30,
          qualityLimitationReason: "bandwidth",
        }),
      ]),
      "voice",
    );
    expect(camera).toMatchObject({
      name: "Kamera",
      count: 2,
      bitrateBps: 2_150_000,
      width: 1280,
      height: 720,
      fps: 30,
      limitedBy: "bandwidth",
    });
  });

  it("names received video, Watch media and mixed codecs plainly", () => {
    const voice = flowGroups(
      snapshot([
        flow({ kind: "video", codec: "AV1" }),
        flow({ kind: "video", codec: "VP9" }),
      ]),
      "voice",
    );
    expect(voice).toHaveLength(1);
    expect(voice[0]).toMatchObject({ name: "Video", codec: null, count: 2 });
    const watch = flowGroups(
      snapshot(
        [
          flow({ kind: "video", source: "watch" }),
          flow({ kind: "audio", source: "watch" }),
        ],
        20,
        "watch",
      ),
      "watch",
    );
    expect(watch.map((group) => group.name)).toEqual(["Live-Ton", "Live"]);
    expect(flowGroups(null, "voice")).toEqual([]);
  });

  it("adds up what goes out and comes in over both connections", () => {
    const voice = snapshot([
      flow({ direction: "send", measuredBitrateBps: 38_000 }),
      flow({ measuredBitrateBps: 40_000 }),
    ]);
    const watch = snapshot(
      [flow({ kind: "video", measuredBitrateBps: 8_000_000 })],
      20,
      "watch",
    );
    expect(directionTotal([voice, watch], "send")).toBe(38_000);
    expect(directionTotal([voice, watch], "recv")).toBe(8_040_000);
    expect(directionTotal([null], "recv")).toBeNull();
  });
});
