// Optional voice diagnostics. Collapsed until someone opens it.
// Figures are clues for a bad call, not a verdict about the cause.

import {
  AUDIO_QUALITY,
  VIDEO_MAX_FPS,
  VIDEO_SEND_BUDGET,
  useMediaSettings,
} from "./settings.ts";
import {
  PHASE_LABEL,
  SOURCE_LABEL,
  buildDiagnosticExport,
  formatBps,
  formatClock,
  formatCount,
  formatMs,
  formatQualityLimit,
  formatRatio,
  useVoiceDiagnostics,
  type ConnectionSnapshot,
  type FlowStats,
} from "./diagnostics.ts";

export function VoiceDiagnostics() {
  const latest = useVoiceDiagnostics((state) => state.latest);
  const samples = useVoiceDiagnostics((state) => state.samples);
  const phases = useVoiceDiagnostics((state) => state.phases);
  const events = useVoiceDiagnostics((state) => state.events);
  const polling = useVoiceDiagnostics((state) => state.polling);
  const quality = useMediaSettings((state) => state.quality);
  const audioCap = AUDIO_QUALITY[quality];
  const measuring = polling.voice || polling.watch;

  return (
    <details className="mt-4 w-full text-left text-sm text-neutral-700 dark:text-neutral-300">
      <summary className="cursor-pointer rounded-md bg-neutral-100 px-3 py-2 font-medium text-neutral-800 hover:bg-neutral-200 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700">
        Sprachdiagnose
      </summary>
      <div className="mt-3 flex flex-col gap-4 rounded-md border border-neutral-200 p-3 dark:border-neutral-700">
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          Messwerte sind Anhaltspunkte. Ein einzelner Wert ist keine bewiesene
          Ursache. Obergrenze meint die eingestellte Grenze, nicht die gemessene
          Bitrate.
        </p>
        <p className="text-xs text-neutral-500 dark:text-neutral-400">
          {measuring ? "Messung läuft." : "Keine laufende Messung."}{" "}
          Eingestellte Audio-Obergrenze: {audioCap.label} ({audioCap.hint}).
          Videobudget {formatBps(VIDEO_SEND_BUDGET)}, höchstens {VIDEO_MAX_FPS}{" "}
          FPS.
        </p>
        <ConnectionBlock
          title="Sprache"
          snapshot={latest?.voice ?? null}
          live={polling.voice}
          showTransport
          filter={(flow) => flow.kind === "audio"}
        />
        <ConnectionBlock
          title="Eigene Videoquellen"
          snapshot={latest?.voice ?? null}
          live={polling.voice}
          filter={(flow) => flow.kind === "video" && flow.direction === "send"}
        />
        <ConnectionBlock
          title="Eingehendes Video (Sprachverbindung)"
          snapshot={latest?.voice ?? null}
          live={polling.voice}
          filter={(flow) => flow.kind === "video" && flow.direction === "recv"}
        />
        <ConnectionBlock
          title="Watch"
          snapshot={latest?.watch ?? null}
          live={polling.watch}
          showTransport
          filter={() => true}
        />
        <section>
          <h3 className="font-medium text-neutral-800 dark:text-neutral-200">
            Verlauf
          </h3>
          {phases.length === 0 ? (
            <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
              noch kein Verlauf
            </p>
          ) : (
            <ul className="mt-1 flex flex-col gap-1 text-xs">
              {phases.map((phase) => (
                <li key={`${phase.at}-${phase.phase}`}>
                  {formatClock(phase.at)} · {PHASE_LABEL[phase.phase]} · Senden{" "}
                  {formatBps(phase.audioSendBps)} · Empfang{" "}
                  {formatBps(phase.audioRecvBps)} · Video{" "}
                  {formatBps(phase.videoSendBps)}
                </li>
              ))}
            </ul>
          )}
          {samples.length > 0 ? (
            <div className="mt-2 max-h-48 overflow-auto">
              <table className="w-full text-left text-xs">
                <thead className="text-neutral-500 dark:text-neutral-400">
                  <tr>
                    <th className="py-1 pr-2 font-medium">Zeit</th>
                    <th className="py-1 pr-2 font-medium">Phase</th>
                    <th className="py-1 pr-2 font-medium">Sprache senden</th>
                    <th className="py-1 pr-2 font-medium">Sprache empfangen</th>
                    <th className="py-1 font-medium">Eigenes Video</th>
                  </tr>
                </thead>
                <tbody>
                  {samples.map((sample, index) => (
                    <tr
                      key={`${sample.at}-${index}`}
                      className="border-t border-neutral-100 dark:border-neutral-800"
                    >
                      <td className="py-1 pr-2">{formatClock(sample.at)}</td>
                      <td className="py-1 pr-2">{PHASE_LABEL[sample.phase]}</td>
                      <td className="py-1 pr-2">
                        {formatBps(sum(sample.voice, "audio", "send"))}
                      </td>
                      <td className="py-1 pr-2">
                        {formatBps(sum(sample.voice, "audio", "recv"))}
                      </td>
                      <td className="py-1">
                        {formatBps(sum(sample.voice, "video", "send"))}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </section>
        <section>
          <h3 className="font-medium text-neutral-800 dark:text-neutral-200">
            Ereignisse
          </h3>
          {events.length === 0 ? (
            <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
              noch keine Ereignisse
            </p>
          ) : (
            <ul className="mt-1 flex flex-col gap-1 text-xs">
              {events.slice(-8).map((event, index) => (
                <li key={`${event.at}-${event.kind}-${event.detail}-${index}`}>
                  {formatClock(event.at)} · {eventLabel(event.kind)} ·{" "}
                  {event.connection === "voice" ? "Sprache" : "Watch"} ·{" "}
                  {event.detail}
                </li>
              ))}
            </ul>
          )}
        </section>
        <button
          type="button"
          onClick={downloadDiagnostics}
          className="self-start rounded-lg bg-neutral-200 px-3 py-2 text-sm font-medium text-neutral-800 transition hover:bg-neutral-300 dark:bg-neutral-700 dark:text-neutral-200 dark:hover:bg-neutral-600"
        >
          JSON herunterladen
        </button>
      </div>
    </details>
  );
}

function ConnectionBlock({
  title,
  snapshot,
  live,
  filter,
  showTransport = false,
}: {
  title: string;
  snapshot: ConnectionSnapshot | null;
  live: boolean;
  filter: (flow: FlowStats) => boolean;
  showTransport?: boolean;
}) {
  const flows = snapshot?.flows.filter(filter) ?? [];
  return (
    <section>
      <h3 className="font-medium text-neutral-800 dark:text-neutral-200">
        {title}
      </h3>
      {showTransport ? (
        snapshot ? (
          <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
            Transport: {transportLabel(snapshot.transport.path)} · RTT{" "}
            {formatMs(snapshot.transport.rttMs)} · Bandbreite senden{" "}
            {formatBps(snapshot.transport.availableOutgoingBps)} · empfangen{" "}
            {formatBps(snapshot.transport.availableIncomingBps)}
            {live ? "" : " · letzte Messung"}
          </p>
        ) : (
          <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
            unbekannt
          </p>
        )
      ) : null}
      {flows.length === 0 ? (
        <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400">
          Keine RTP-Statistik für diesen Bereich.
        </p>
      ) : (
        <ul className="mt-2 flex flex-col gap-2">
          {flows.map((flow, index) => (
            <li
              key={`${flow.direction}-${flow.kind}-${flow.source}-${index}`}
              className="rounded-md bg-neutral-50 px-2 py-2 text-xs dark:bg-neutral-900"
            >
              <FlowLine flow={flow} />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function FlowLine({ flow }: { flow: FlowStats }) {
  const direction = flow.direction === "send" ? "Senden" : "Empfangen";
  return (
    <div className="flex flex-col gap-0.5">
      <p className="font-medium text-neutral-800 dark:text-neutral-200">
        {SOURCE_LABEL[flow.source]} · {direction}
      </p>
      <p>
        Codec: {flow.codec ?? "unbekannt"} · Bitrate:{" "}
        {formatBps(flow.measuredBitrateBps)}
        {flow.configuredMaxBitrateBps !== null
          ? ` · Obergrenze: ${formatBps(flow.configuredMaxBitrateBps)}`
          : ""}
      </p>
      <p>
        Paketverlust: {formatRatio(flow.packetLoss)}
        {flow.packetsLost !== null
          ? ` (${formatCount(flow.packetsLost)} Pakete)`
          : ""}{" "}
        · Jitter: {formatMs(flow.jitterMs)} · RTT: {formatMs(flow.rttMs)}
      </p>
      {flow.kind === "audio" && flow.direction === "recv" ? (
        <p>
          Ersetzte Samples:{" "}
          {flow.concealedRatio === null ||
          flow.concealedSamples === null ||
          flow.totalSamplesReceived === null
            ? "unbekannt"
            : `${formatRatio(flow.concealedRatio)} (${formatCount(flow.concealedSamples)} / ${formatCount(flow.totalSamplesReceived)})`}
        </p>
      ) : null}
      {flow.kind === "video" ? (
        <p>
          {flow.width !== null && flow.height !== null
            ? `${flow.width}×${flow.height}`
            : "Auflösung unbekannt"}{" "}
          ·{" "}
          {flow.fps !== null ? `${formatCount(flow.fps)} FPS` : "FPS unbekannt"}
          {flow.configuredMaxFps !== null
            ? ` · FPS-Obergrenze: ${flow.configuredMaxFps}`
            : ""}{" "}
          · Qualitätslimit: {formatQualityLimit(flow.qualityLimitationReason)}
        </p>
      ) : null}
    </div>
  );
}

function sum(
  snapshot: ConnectionSnapshot | null,
  kind: "audio" | "video",
  direction: "send" | "recv",
): number | null {
  const flows =
    snapshot?.flows.filter(
      (flow) => flow.kind === kind && flow.direction === direction,
    ) ?? [];
  if (flows.length === 0) return null;
  if (flows.some((flow) => flow.measuredBitrateBps === null)) return null;
  return flows.reduce(
    (total, flow) => total + (flow.measuredBitrateBps ?? 0),
    0,
  );
}

function transportLabel(path: "direct" | "turn" | null): string {
  if (path === "direct") return "direkt";
  if (path === "turn") return "TURN";
  return "unbekannt";
}

function eventLabel(kind: string): string {
  switch (kind) {
    case "stream-start":
      return "Streamstart";
    case "stream-stop":
      return "Streamstopp";
    case "device-change":
      return "Gerätewechsel";
    case "sdp-error":
      return "SDP-Fehler";
    case "ice-error":
      return "ICE-Fehler";
    case "recovery":
      return "Recovery";
    default:
      return kind;
  }
}

function downloadDiagnostics(): void {
  const payload = buildDiagnosticExport();
  const json = JSON.stringify(payload, null, 2);
  const blob = new Blob([json], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `gelabber-diagnose-${payload.exportedAt.replace(/[:.]/g, "-")}.json`;
  link.click();
  URL.revokeObjectURL(url);
}
