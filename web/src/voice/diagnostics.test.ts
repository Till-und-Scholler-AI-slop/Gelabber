import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import packageJson from "../../package.json" with { type: "json" };

import { resetSessionForTests, useSession } from "../auth/session.ts";
import { APP_VERSION } from "../version.ts";
import {
  DIAGNOSTIC_LIMITS,
  applyStatsReport,
  attachDiagnostics,
  buildDiagnosticExport,
  detachDiagnostics,
  diagnosticsPolling,
  diffCounter,
  installDiagnosticsLogoutReset,
  noteDiagnosticEvent,
  reduceConnection,
  resetDiagnostics,
  sanitizeDetail,
  statsEntriesFromReport,
  useVoiceDiagnostics,
  type Caps,
  type StatsEntry,
} from "./diagnostics.ts";
import { resetMediaSettingsForTests, useMediaSettings } from "./settings.ts";

const caps: Caps = {
  audioMaxBitrate: 64_000,
  videoSendBudget: 2_500_000,
  videoMaxFps: 30,
};

function entry(partial: StatsEntry): StatsEntry {
  return partial;
}

describe("voice diagnostics", () => {
  beforeEach(() => {
    installDiagnosticsLogoutReset((listener) => {
      useSession.subscribe(listener);
    });
    resetDiagnostics();
    resetMediaSettingsForTests();
  });

  afterEach(() => {
    resetDiagnostics();
    resetMediaSettingsForTests();
    resetSessionForTests();
    vi.useRealTimers();
  });

  it("computes bitrate and loss from counter deltas", () => {
    expect(diffCounter(undefined, 0)).toEqual({ delta: null, reset: false });
    expect(diffCounter(1_000, 5_000)).toEqual({ delta: 4_000, reset: false });
    expect(diffCounter(undefined, undefined)).toEqual({
      delta: null,
      reset: false,
    });

    const previous = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: new Map(),
      entries: [
        entry({
          id: "opus",
          type: "codec",
          mimeType: "audio/opus",
        }),
        entry({
          id: "out",
          type: "outbound-rtp",
          kind: "audio",
          codecId: "opus",
          timestamp: 1_000,
          bytesSent: 0,
          packetsSent: 0,
        }),
        entry({
          id: "remote",
          type: "remote-inbound-rtp",
          kind: "audio",
          localId: "out",
          timestamp: 1_000,
          packetsLost: 0,
          jitter: 0.01,
          roundTripTime: 0.04,
        }),
        entry({
          id: "in",
          type: "inbound-rtp",
          kind: "audio",
          codecId: "opus",
          timestamp: 1_000,
          bytesReceived: 0,
          packetsReceived: 0,
          packetsLost: 0,
          jitter: 0.012,
          concealedSamples: 0,
          totalSamplesReceived: 0,
        }),
      ],
    });
    const firstSend = previous.snapshot.flows.find(
      (flow) => flow.direction === "send",
    );
    expect(firstSend?.measuredBitrateBps).toBeNull();
    expect(firstSend?.codec).toBe("opus");
    expect(firstSend?.configuredMaxBitrateBps).toBe(64_000);

    const next = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: previous.next,
      entries: [
        entry({ id: "opus", type: "codec", mimeType: "audio/opus" }),
        entry({
          id: "out",
          type: "outbound-rtp",
          kind: "audio",
          codecId: "opus",
          timestamp: 3_000,
          bytesSent: 16_000,
          packetsSent: 100,
        }),
        entry({
          id: "remote",
          type: "remote-inbound-rtp",
          kind: "audio",
          localId: "out",
          timestamp: 3_000,
          packetsLost: 2,
          jitter: 0.01,
          roundTripTime: 0.04,
        }),
        entry({
          id: "in",
          type: "inbound-rtp",
          kind: "audio",
          codecId: "opus",
          timestamp: 3_000,
          bytesReceived: 8_000,
          packetsReceived: 98,
          packetsLost: 2,
          jitter: 0.02,
          concealedSamples: 50,
          totalSamplesReceived: 1_000,
        }),
      ],
    });
    const send = next.snapshot.flows.find((flow) => flow.direction === "send");
    const recv = next.snapshot.flows.find((flow) => flow.direction === "recv");
    expect(send?.measuredBitrateBps).toBe(64_000);
    expect(send?.configuredMaxBitrateBps).toBe(64_000);
    expect(send?.packetLoss).toBeCloseTo(2 / 100);
    expect(send?.packetsLost).toBe(2);
    expect(send?.jitterMs).toBeCloseTo(10);
    expect(send?.rttMs).toBeCloseTo(40);
    expect(recv?.measuredBitrateBps).toBe(32_000);
    expect(recv?.packetLoss).toBeCloseTo(2 / 100);
    expect(recv?.jitterMs).toBeCloseTo(20);
    expect(recv?.concealedSamples).toBe(50);
    expect(recv?.totalSamplesReceived).toBe(1_000);
    expect(recv?.concealedRatio).toBeCloseTo(0.05);
    expect(recv?.configuredMaxBitrateBps).toBeNull();
  });

  it("treats a decreasing counter as a reset and continues from the new baseline", () => {
    const first = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: new Map(),
      entries: [
        entry({
          id: "out",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 1_000,
          bytesSent: 5_000,
          packetsSent: 10,
        }),
      ],
    });
    const reset = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: first.next,
      entries: [
        entry({
          id: "out",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 2_000,
          bytesSent: 100,
          packetsSent: 1,
        }),
      ],
    });
    expect(diffCounter(5_000, 100)).toEqual({ delta: null, reset: true });
    expect(reset.snapshot.flows[0]?.measuredBitrateBps).toBeNull();
    expect(reset.snapshot.flows[0]?.packetsLost).toBeNull();

    const after = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: reset.next,
      entries: [
        entry({
          id: "out",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 3_000,
          bytesSent: 1_100,
          packetsSent: 4,
        }),
      ],
    });
    expect(after.snapshot.flows[0]?.measuredBitrateBps).toBe(8_000);
  });

  it("starts a new baseline when the stat id changes", () => {
    const first = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: new Map(),
      entries: [
        entry({
          id: "out-a",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 1_000,
          bytesSent: 8_000,
        }),
      ],
    });
    const replaced = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: first.next,
      entries: [
        entry({
          id: "out-b",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 2_000,
          bytesSent: 16_000,
        }),
      ],
    });
    expect(replaced.snapshot.flows[0]?.measuredBitrateBps).toBeNull();
    const settled = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: replaced.next,
      entries: [
        entry({
          id: "out-b",
          type: "outbound-rtp",
          kind: "audio",
          timestamp: 3_000,
          bytesSent: 32_000,
        }),
      ],
    });
    expect(settled.snapshot.flows[0]?.measuredBitrateBps).toBe(128_000);
    expect(settled.next.has("out-a")).toBe(false);
  });

  it("keeps missing browser fields unknown instead of zero", () => {
    const seeded = reduceConnection({
      role: "voice",
      caps,
      videoSources: { cam: "camera" },
      previous: new Map(),
      entries: [
        entry({
          id: "in",
          type: "inbound-rtp",
          kind: "audio",
          timestamp: 1_000,
          bytesReceived: 0,
          packetsReceived: 100,
        }),
        entry({
          id: "cam-out",
          type: "outbound-rtp",
          kind: "video",
          trackIdentifier: "cam",
          timestamp: 1_000,
          bytesSent: 0,
          framesDecoded: 0,
        }),
      ],
    });
    const reduced = reduceConnection({
      role: "voice",
      caps,
      videoSources: { cam: "camera" },
      previous: seeded.next,
      entries: [
        entry({
          id: "in",
          type: "inbound-rtp",
          kind: "audio",
          timestamp: 2_000,
          bytesReceived: 4_000,
          packetsReceived: 150,
        }),
        entry({
          id: "cam-out",
          type: "outbound-rtp",
          kind: "video",
          trackIdentifier: "cam",
          timestamp: 2_000,
          bytesSent: 100_000,
          framesDecoded: 30,
        }),
        entry({
          id: "pair",
          type: "candidate-pair",
          selected: true,
          state: "succeeded",
          localCandidateId: "local",
          remoteCandidateId: "remote",
        }),
        entry({ id: "local", type: "local-candidate", candidateType: "host" }),
      ],
    });
    const recv = reduced.snapshot.flows.find(
      (flow) => flow.direction === "recv",
    );
    expect(recv?.measuredBitrateBps).toBe(32_000);
    expect(recv?.packetLoss).toBeNull();
    expect(recv?.packetsLost).toBeNull();
    expect(recv?.jitterMs).toBeNull();
    expect(recv?.concealedSamples).toBeNull();
    expect(recv?.totalSamplesReceived).toBeNull();
    expect(recv?.concealedRatio).toBeNull();
    const video = reduced.snapshot.flows.find((flow) => flow.kind === "video");
    expect(video?.source).toBe("camera");
    expect(video?.width).toBeNull();
    expect(video?.height).toBeNull();
    expect(video?.fps).toBe(30);
    expect(video?.qualityLimitationReason).toBeNull();
    expect(video?.configuredMaxBitrateBps).toBe(2_500_000);
    expect(video?.configuredMaxFps).toBe(30);
    expect(reduced.snapshot.transport.path).toBeNull();
    expect(reduced.snapshot.transport.availableOutgoingBps).toBeNull();
    expect(reduced.snapshot.transport.availableIncomingBps).toBeNull();
    expect(reduced.snapshot.transport.rttMs).toBeNull();
  });

  it("labels direct and TURN paths only when both candidates are known", () => {
    const turn = reduceConnection({
      role: "watch",
      caps,
      videoSources: {},
      previous: new Map(),
      entries: [
        entry({
          id: "pair",
          type: "candidate-pair",
          selected: true,
          state: "succeeded",
          localCandidateId: "local",
          remoteCandidateId: "remote",
          currentRoundTripTime: 0.08,
          availableOutgoingBitrate: 1_200_000,
        }),
        entry({ id: "local", type: "local-candidate", candidateType: "relay" }),
        entry({
          id: "remote",
          type: "remote-candidate",
          candidateType: "host",
        }),
        entry({
          id: "vin",
          type: "inbound-rtp",
          kind: "video",
          timestamp: 5_000,
          frameWidth: 1280,
          frameHeight: 720,
          framesPerSecond: 24,
        }),
      ],
    });
    expect(turn.snapshot.transport.path).toBe("turn");
    expect(turn.snapshot.transport.rttMs).toBeCloseTo(80);
    expect(turn.snapshot.transport.availableOutgoingBps).toBe(1_200_000);
    expect(turn.snapshot.transport.availableIncomingBps).toBeNull();
    const inbound = turn.snapshot.flows[0];
    expect(inbound?.source).toBe("watch");
    expect(inbound?.width).toBe(1280);
    expect(inbound?.height).toBe(720);
    expect(inbound?.fps).toBe(24);
    expect(inbound?.measuredBitrateBps).toBeNull();

    const direct = reduceConnection({
      role: "voice",
      caps,
      videoSources: {},
      previous: new Map(),
      entries: [
        entry({
          id: "transport",
          type: "transport",
          selectedCandidatePairId: "pair",
        }),
        entry({
          id: "pair",
          type: "candidate-pair",
          state: "succeeded",
          localCandidateId: "local",
          remoteCandidateId: "remote",
        }),
        entry({ id: "local", type: "local-candidate", candidateType: "srflx" }),
        entry({
          id: "remote",
          type: "remote-candidate",
          candidateType: "host",
        }),
      ],
    });
    expect(direct.snapshot.transport.path).toBe("direct");
  });

  it("does not copy addresses, candidates, or SDP out of getStats", () => {
    const report = new Map<string, Record<string, unknown>>([
      [
        "local",
        {
          id: "local",
          type: "local-candidate",
          candidateType: "host",
          address: "203.0.113.9",
          ip: "203.0.113.9",
          port: 9,
          candidate: "candidate:1 1 UDP 1 203.0.113.9 9 typ host",
          url: "stun:203.0.113.9:3478",
          usernameFragment: "secretfrag",
          relatedAddress: "198.51.100.4",
          sdp: "v=0",
        },
      ],
    ]);
    const entries = statsEntriesFromReport(report);
    const json = JSON.stringify(entries);
    expect(entries[0]?.candidateType).toBe("host");
    expect(json).not.toContain("203.0.113.9");
    expect(json).not.toContain("198.51.100.4");
    expect(json).not.toContain("candidate:");
    expect(json).not.toContain("secretfrag");
    expect(json).not.toContain("v=0");
    expect(sanitizeDetail("failed 203.0.113.9 candidate:abc")).not.toContain(
      "203.0.113",
    );
    expect(
      sanitizeDetail("v=0\r\na=candidate:1 1 UDP 1 203.0.113.9 9 typ host"),
    ).toBe("redacted");
    expect(sanitizeDetail("Bearer session.token")).toBe("unbekannt");
  });

  it("keeps a 36-character track id linked through mediaSourceId", () => {
    const track = "a1111111-b222-c333-d444-e55555555555";
    const report = new Map<string, Record<string, unknown>>([
      [
        "out-cam",
        {
          id: "out-cam",
          type: "outbound-rtp",
          kind: "video",
          mediaSourceId: track,
          timestamp: 2_000,
          bytesSent: 80_000,
        },
      ],
      [
        track,
        {
          id: track,
          type: "media-source",
          kind: "video",
          trackIdentifier: track,
          timestamp: 2_000,
        },
      ],
    ]);
    const entries = statsEntriesFromReport(report);
    const outbound = entries.find((item) => item.id === "out-cam");
    const source = entries.find((item) => item.id === track);
    expect(outbound?.mediaSourceId).toBe(track);
    expect(source?.trackIdentifier).toBe(track);
    const reduced = reduceConnection({
      role: "voice",
      caps,
      videoSources: { [track]: "camera" },
      previous: new Map(),
      entries,
    });
    expect(
      reduced.snapshot.flows.find((flow) => flow.kind === "video")?.source,
    ).toBe("camera");
  });

  it("exports context without secrets and keeps history bounded", () => {
    useMediaSettings.getState().patch({
      audioInputId: "secret-device-id-xyz",
      quality: "high",
    });
    noteDiagnosticEvent({
      kind: "sdp-error",
      connection: "voice",
      detail: "v=0\r\no=- 1 IN IP4 203.0.113.8\r\n",
      at: 1_700_000_000_000,
    });
    noteDiagnosticEvent({
      kind: "recovery",
      connection: "voice",
      detail: "connected",
      at: 1_700_000_001_000,
    });
    noteDiagnosticEvent({
      kind: "ice-error",
      connection: "watch",
      detail: "disconnected at 198.51.100.8",
      at: 1_700_000_002_000,
    });
    expect(useVoiceDiagnostics.getState().events[0]?.detail).toBe("redacted");
    expect(
      useVoiceDiagnostics
        .getState()
        .events.some((event) => event.kind === "recovery"),
    ).toBe(true);
    expect(
      useVoiceDiagnostics
        .getState()
        .events.some((event) => event.detail.includes("198.51.100")),
    ).toBe(false);
    for (let index = 0; index < DIAGNOSTIC_LIMITS.samples + 12; index += 1) {
      applyStatsReport(
        "voice",
        [
          entry({
            id: "out",
            type: "outbound-rtp",
            kind: "audio",
            timestamp: index * 1_000,
            bytesSent: index * 1_000,
          }),
        ],
        {
          streaming: false,
          caps,
          videoSources: {},
          now: 1_700_000_010_000 + index,
        },
      );
    }
    for (let index = 0; index < DIAGNOSTIC_LIMITS.events; index += 1) {
      noteDiagnosticEvent({
        kind: index % 2 === 0 ? "stream-start" : "stream-stop",
        connection: "voice",
        detail: "camera",
        streaming: index % 2 === 0,
        at: 1_700_000_100_000 + index,
      });
    }
    noteDiagnosticEvent({
      kind: "sdp-error",
      connection: "voice",
      detail:
        "v=0\r\nc=IN IP4 203.0.113.8\r\na=candidate:9 1 UDP 1 203.0.113.8 9 typ host",
      at: 1_700_000_200_000,
    });
    noteDiagnosticEvent({
      kind: "ice-error",
      connection: "voice",
      detail: "disconnected at 198.51.100.8",
      at: 1_700_000_200_001,
    });
    const state = useVoiceDiagnostics.getState();
    expect(state.samples).toHaveLength(DIAGNOSTIC_LIMITS.samples);
    expect(state.events).toHaveLength(DIAGNOSTIC_LIMITS.events);
    expect(state.phases.length).toBeLessThanOrEqual(DIAGNOSTIC_LIMITS.phases);
    expect(state.events.at(-2)?.detail).toBe("redacted");
    expect(state.events.at(-1)?.detail).not.toContain("198.51.100");

    const exported = buildDiagnosticExport(
      new Date("2026-09-22T08:00:00.000Z"),
    );
    const json = JSON.stringify(exported);
    expect(exported.appVersion).toBe(APP_VERSION);
    expect(exported.appVersion).toBe(packageJson.version);
    expect(exported.exportedAt).toBe("2026-09-22T08:00:00.000Z");
    expect(exported.browser.userAgent.length).toBeGreaterThan(0);
    expect(exported.settings.audioQuality).toBe("high");
    expect(exported.settings.audioMaxBitrate).toBe(128_000);
    expect(exported.settings.customAudioInput).toBe(true);
    expect(exported.settings.videoMaxFps).toBe(30);
    expect(json).not.toContain("secret-device-id-xyz");
    expect(json).not.toContain("audioInputId");
    expect(json).not.toContain("203.0.113.8");
    expect(json).not.toContain("198.51.100");
    expect(json).not.toContain("v=0");
    expect(json).not.toContain("candidate:");
    expect(json).not.toMatch(/cookie|bearer|password/i);
    expect(exported.note).toContain("keine bewiesene");
  });

  it("records voice-only, stream on, and stream off in order", () => {
    attachDiagnostics({
      role: "voice",
      getReport: async () => null,
      streaming: () => false,
    });
    noteDiagnosticEvent({
      kind: "stream-start",
      connection: "voice",
      detail: "screen",
      streaming: true,
      at: 2_000,
    });
    noteDiagnosticEvent({
      kind: "stream-stop",
      connection: "voice",
      detail: "screen",
      streaming: false,
      at: 3_000,
    });
    expect(
      useVoiceDiagnostics.getState().phases.map((phase) => phase.phase),
    ).toEqual(["voice-only", "stream-on", "stream-off"]);
  });

  it("keeps an active camera when a watch sample arrives", () => {
    let streaming = true;
    attachDiagnostics({
      role: "voice",
      streaming: () => streaming,
      getReport: async () => null,
    });
    attachDiagnostics({
      role: "watch",
      streaming: () => false,
      getReport: async () => null,
    });
    const on = applyStatsReport("watch", [], {
      streaming: false,
      caps,
      videoSources: {},
      now: 4_000,
    });
    expect(on.phase).toBe("stream-on");
    expect(
      useVoiceDiagnostics.getState().phases.map((phase) => phase.phase),
    ).toEqual(["stream-on"]);

    streaming = false;
    const off = applyStatsReport("watch", [], {
      streaming: false,
      caps,
      videoSources: {},
      now: 5_000,
    });
    expect(off.phase).toBe("stream-off");
    expect(
      useVoiceDiagnostics.getState().phases.map((phase) => phase.phase),
    ).toEqual(["stream-on", "stream-off"]);
  });

  it("stops the old interval on leave, rejoin, and logout", async () => {
    vi.useFakeTimers();
    let calls = 0;
    let releaseFirst: (value: StatsEntry[] | null) => void = () => undefined;
    const first = new Promise<StatsEntry[] | null>((resolve) => {
      releaseFirst = resolve;
    });
    attachDiagnostics({
      role: "voice",
      getReport: () => {
        calls += 1;
        return calls === 1 ? first : Promise.resolve([]);
      },
    });
    expect(diagnosticsPolling().voice).toBe(true);
    detachDiagnostics("voice");
    releaseFirst([
      entry({
        id: "out",
        type: "outbound-rtp",
        kind: "audio",
        timestamp: 1,
        bytesSent: 50_000,
      }),
    ]);
    await first;
    await vi.advanceTimersByTimeAsync(0);
    expect(useVoiceDiagnostics.getState().samples).toHaveLength(0);

    calls = 0;
    attachDiagnostics({
      role: "voice",
      getReport: async () => {
        calls += 1;
        return [];
      },
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(calls).toBe(1);
    await vi.advanceTimersByTimeAsync(DIAGNOSTIC_LIMITS.intervalMs);
    expect(calls).toBe(2);
    detachDiagnostics("voice");
    attachDiagnostics({
      role: "voice",
      getReport: async () => {
        calls += 1;
        return [];
      },
    });
    await vi.advanceTimersByTimeAsync(DIAGNOSTIC_LIMITS.intervalMs * 4);
    await Promise.resolve();
    expect(diagnosticsPolling().voice).toBe(true);
    const afterRejoin = calls;
    await vi.advanceTimersByTimeAsync(DIAGNOSTIC_LIMITS.intervalMs);
    expect(calls).toBeGreaterThan(afterRejoin);
    expect(calls - afterRejoin).toBe(1);

    useSession.setState({
      status: "authenticated",
      user: {
        id: "u1",
        email: "ada@example.com",
        name: "Ada",
        avatar_url: null,
        created_at: "2026-01-01T00:00:00.000Z",
      },
    });
    useSession.setState({ status: "anonymous", user: null });
    expect(diagnosticsPolling().voice).toBe(false);
    expect(diagnosticsPolling().watch).toBe(false);
    expect(useVoiceDiagnostics.getState().samples).toHaveLength(0);
    expect(JSON.stringify(buildDiagnosticExport())).not.toContain(
      "ada@example.com",
    );
    await vi.advanceTimersByTimeAsync(DIAGNOSTIC_LIMITS.intervalMs * 3);
    expect(calls).toBeGreaterThan(0);
    const stopped = calls;
    await vi.advanceTimersByTimeAsync(DIAGNOSTIC_LIMITS.intervalMs * 3);
    expect(calls).toBe(stopped);
  });
});
