import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

// A static render reads a store's initial state; the components here read
// the measurement a test sets.
vi.mock("./diagnostics.ts", async (original) => {
  const actual = await original<typeof import("./diagnostics.ts")>();
  const store = actual.useVoiceDiagnostics;
  const live = <T,>(
    selector: (state: ReturnType<typeof store.getState>) => T,
  ) => selector(store.getState());
  return { ...actual, useVoiceDiagnostics: Object.assign(live, store) };
});

import { ConnectionStatus } from "./ConnectionStatus.tsx";
import {
  resetDiagnostics,
  useVoiceDiagnostics,
  type ConnectionSnapshot,
  type DiagnosticSample,
  type FlowStats,
} from "./diagnostics.ts";
import { VoiceDiagnostics } from "./VoiceDiagnostics.tsx";

function flow(partial: Partial<FlowStats>): FlowStats {
  return {
    direction: "recv",
    kind: "audio",
    source: "voice",
    codec: "opus",
    measuredBitrateBps: 40_000,
    configuredMaxBitrateBps: null,
    configuredMaxFps: null,
    senderPriority: null,
    senderNetworkPriority: null,
    packetLoss: 0,
    packetsLost: 0,
    jitterMs: 2,
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

function measure(rttMs: number, flows: FlowStats[]): void {
  const voice: ConnectionSnapshot = {
    role: "voice",
    transport: {
      path: "direct",
      rttMs,
      availableIncomingBps: null,
      availableOutgoingBps: 352_000,
    },
    flows,
  };
  const sample: DiagnosticSample = {
    at: "2026-10-28T20:00:00.000Z",
    phase: "voice-only",
    caps: { audioMaxBitrate: null, videoSendBudget: null, videoMaxFps: 60 },
    voice,
    watch: null,
  };
  useVoiceDiagnostics.setState({
    latest: sample,
    samples: [sample, sample],
    polling: { voice: true, watch: false },
  });
}

afterEach(() => resetDiagnostics());

describe("voice diagnostics", () => {
  it("answers first, then lists what is sent and received", () => {
    measure(14, [
      flow({ direction: "send", measuredBitrateBps: 38_000 }),
      flow({}),
      flow({ packetLoss: 0.06 }),
    ]);
    const html = renderToStaticMarkup(<VoiceDiagnostics />);
    // The worst voice flow decides, as on the call bar.
    expect(html).toContain("Verbindung schwach</h3>");
    expect(html).toContain("Direkt verbunden, hoher Verlust (6 %).");
    expect(html.indexOf("Verbindung schwach")).toBeLessThan(
      html.indexOf("Übertragung"),
    );
    // Everyone's voice is one row, with the worst loss marked.
    expect(html).toContain("Sprache<small> ×2</small>");
    expect(html).toMatch(/data-tone="weak">6,0 %/);
    // No row and no heading for media that is not there.
    expect(html).not.toContain("Keine RTP-Statistik für diesen Bereich");
    expect(html).not.toContain("Eigene Videoquellen");
    // The raw figures wait behind a closed disclosure.
    expect(html).toMatch(/<details class="media-disclosure"><summary>/);
    expect(html).toContain("Diagnose herunterladen");
  });

  it("points to the economy mode when sending loses packets", () => {
    measure(340, [flow({ direction: "send", packetLoss: 0.03 }), flow({})]);
    expect(renderToStaticMarkup(<VoiceDiagnostics />)).toContain(
      "Zum Sparmodus",
    );
    // A slow but clean call is no case for it.
    measure(340, [flow({ direction: "send", packetLoss: 0 }), flow({})]);
    expect(renderToStaticMarkup(<VoiceDiagnostics />)).not.toContain(
      "Zum Sparmodus",
    );
  });

  it("says when nothing is measured instead of drawing empty tables", () => {
    const html = renderToStaticMarkup(<VoiceDiagnostics />);
    expect(html).toContain("Keine laufende Messung</h3>");
    expect(html).toContain("Werte gibt es, sobald du in einem Anruf bist.");
    expect(html).not.toContain('<table class="connection-flows"');
  });

  it("keeps the popover to the bars, two figures and the trace", () => {
    measure(340, [flow({ packetLoss: 0.01 })]);
    const html = renderToStaticMarkup(<ConnectionStatus compact />);
    expect(html).toContain('data-level="2"');
    expect(html).toContain("Verbindung schwach</h3>");
    expect(html).toContain("direkt verbunden");
    expect(html).toContain("Ping");
    expect(html).not.toContain("Verbunden</dt>");
    // 30 bars, the two measured ones on the right.
    expect(html.match(/<span data-tone="weak"/g)).toHaveLength(2);
  });
});
