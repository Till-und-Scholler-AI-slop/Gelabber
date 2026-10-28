// Voice diagnostics, the settings dialog's "Diagnose" tab; the call bar's
// connection popover links here. The status on top answers whether the
// call is fine; the table shows what is being sent and received; the raw
// figures wait under "Für die Fehlersuche". They are clues, not a verdict
// about the cause.

import { Button } from "../components/ui/Button.tsx";
import { useAudioProcessing } from "./audioProcessing.ts";
import { ConnectionStatus } from "./ConnectionStatus.tsx";
import { flowGroups, lossTone, type FlowGroup } from "./connectionSummary.ts";
import { isDesktopApp } from "./native/bridge.ts";
import {
  audioBitrate,
  VIDEO_MAX_FPS,
  videoSendBudget,
  useMediaSettings,
} from "./settings.ts";
import {
  CALL_QUALITY_LIMITS,
  PHASE_LABEL,
  SOURCE_LABEL,
  buildDiagnosticExport,
  callQuality,
  formatBps,
  formatClock,
  formatCount,
  formatMs,
  formatQualityLimit,
  formatRatio,
  useVoiceDiagnostics,
  type ConnectionSnapshot,
  type FlowStats,
  type VideoChoice,
} from "./diagnostics.ts";
import "./connection.css";
import "./quality.css";

const onOff = (value: boolean | null | undefined) =>
  value === null || value === undefined ? "–" : value ? "an" : "aus";

const yesNo = (value: boolean | null | undefined) =>
  value === null || value === undefined ? "–" : value ? "ja" : "nein";

/** A figure the browser did not report reads as a dash, not "unbekannt". */
const known = (text: string) => (text === "unbekannt" ? "–" : text);

export function VoiceDiagnostics() {
  return (
    <section aria-label="Sprachdiagnose" className="voice-diagnostics">
      <ConnectionStatus>
        <UploadTip />
      </ConnectionStatus>
      <Flows />
      <Microphone />
      <Debug />
      <footer className="connection-footer">
        <p>
          Messwerte sind Anhaltspunkte. Ein einzelner Wert ist keine bewiesene
          Ursache.
        </p>
        <Button
          size="sm"
          variant="ghost"
          onClick={downloadDiagnostics}
          title="Speichert die Messwerte als JSON-Datei, etwa für einen Fehlerbericht."
        >
          Diagnose herunterladen
        </Button>
      </footer>
    </section>
  );
}

/** Loss on the way out of a weak call: the economy mode is the setting
 * that helps, and it sits on the video tab. */
function UploadTip() {
  const voice = useVoiceDiagnostics((state) => state.latest?.voice ?? null);
  const measuring = useVoiceDiagnostics((state) => state.polling.voice);
  const economy = useMediaSettings((s) => s.economyMode);
  const openDialog = useMediaSettings((s) => s.openDialog);
  const quality = measuring ? callQuality(voice) : null;
  const sendLoss = Math.max(
    0,
    ...(voice?.flows ?? [])
      .filter((flow) => flow.direction === "send")
      .map((flow) => flow.packetLoss ?? 0),
  );
  if (
    economy ||
    !quality ||
    quality.level > 2 ||
    sendLoss < CALL_QUALITY_LIMITS.loss[0]
  )
    return null;
  return (
    <div className="connection-tip">
      <p>Viel Verlust beim Senden? Der Sparmodus begrenzt deinen Upload.</p>
      <Button size="sm" onClick={() => openDialog("video")}>
        Zum Sparmodus
      </Button>
    </div>
  );
}

function Flows() {
  const latest = useVoiceDiagnostics((state) => state.latest);
  const polling = useVoiceDiagnostics((state) => state.polling);
  const mediaSettings = useMediaSettings();
  const audioCap = audioBitrate(mediaSettings);
  const videoCap = videoSendBudget(mediaSettings);
  const groups = [
    ...flowGroups(latest?.voice ?? null, "voice"),
    ...flowGroups(latest?.watch ?? null, "watch"),
  ];
  const measuring = polling.voice || polling.watch;
  const limits = [
    audioCap === null ? null : `Sprache ${formatBps(audioCap)}`,
    videoCap === null ? null : `Video ${formatBps(videoCap)}`,
  ].filter(Boolean);
  return (
    <section aria-labelledby="connection-flows-title">
      <div className="connection-block-head">
        <h3 id="connection-flows-title">Übertragung</h3>
        {groups.length > 0 && !measuring ? <span>letzte Messung</span> : null}
      </div>
      {groups.length === 0 ? (
        <p className="connection-empty">
          {measuring
            ? "Noch nichts gemessen. Die ersten Werte kommen nach zwei Sekunden."
            : "Keine laufende Messung. Werte gibt es, sobald du in einem Anruf bist."}
        </p>
      ) : (
        <div className="connection-flows-box">
          <table className="connection-flows">
            <thead>
              <tr>
                <th scope="col">Spur</th>
                <th scope="col" className="is-codec">
                  Codec
                </th>
                <th scope="col" className="is-number">
                  Bitrate
                </th>
                <th scope="col" className="is-number">
                  Verlust
                </th>
                <th scope="col" className="is-number">
                  Jitter
                </th>
              </tr>
            </thead>
            <tbody>
              {groups.map((group) => (
                <FlowRow key={group.key} group={group} />
              ))}
            </tbody>
          </table>
        </div>
      )}
      <p className="connection-note">
        {limits.length === 0
          ? "Gelabber setzt keine Bitratengrenze."
          : `Obergrenze${limits.length > 1 ? "n" : ""}: ${limits.join(", ")}.`}{" "}
        Video höchstens {VIDEO_MAX_FPS} FPS.
      </p>
    </section>
  );
}

function FlowRow({ group }: { group: FlowGroup }) {
  const send = group.direction === "send";
  const count =
    group.count < 2
      ? null
      : send && group.kind === "video"
        ? `${group.count} Schichten`
        : `×${group.count}`;
  const picture =
    group.kind === "video"
      ? [
          group.width !== null && group.height !== null
            ? `${group.width}×${group.height}`
            : null,
          group.fps !== null ? `${formatCount(group.fps)} FPS` : null,
        ].filter(Boolean)
      : [];
  return (
    <tr>
      <th scope="row">
        <span className="connection-dir" aria-hidden="true">
          {send ? "↑" : "↓"}
        </span>
        <span className="sr-only">{send ? "Senden: " : "Empfangen: "}</span>
        {group.name}
        {count ? <small> {count}</small> : null}
        {picture.length > 0 || group.limitedBy ? (
          <span className="connection-flow-sub">
            {picture.length > 0 ? <span>{picture.join(" · ")}</span> : null}
            {group.limitedBy ? (
              <span className="connection-chip">
                gebremst durch {formatQualityLimit(group.limitedBy)}
              </span>
            ) : null}
          </span>
        ) : null}
      </th>
      <td className="is-codec">{group.codec ?? "–"}</td>
      <td className="is-number">{known(formatBps(group.bitrateBps))}</td>
      <td
        className="is-number"
        data-tone={group.loss === null ? undefined : lossTone(group.loss)}
      >
        {known(formatRatio(group.loss))}
      </td>
      <td className="is-number">{known(formatMs(group.jitterMs))}</td>
    </tr>
  );
}

function Microphone() {
  const processing = useAudioProcessing();
  const desktop = isDesktopApp();
  const format = [
    processing.sampleRate === null
      ? null
      : `${(processing.sampleRate / 1000).toLocaleString("de-DE")} kHz`,
    processing.channels === null
      ? null
      : processing.channels === 1
        ? "Mono"
        : processing.channels === 2
          ? "Stereo"
          : `${processing.channels} Kanäle`,
  ].filter(Boolean);
  return (
    <section aria-labelledby="connection-mic-title">
      <div className="connection-block-head">
        <h3 id="connection-mic-title">Mikrofon</h3>
      </div>
      <dl className="connection-facts">
        <div className="is-wide">
          <dt>Verarbeitung</dt>
          <dd>{processing.message}</dd>
        </div>
        <div>
          <dt>Format</dt>
          <dd>{format.length > 0 ? format.join(", ") : "–"}</dd>
        </div>
        <div>
          <dt>Echo-Unterdrückung</dt>
          <dd>{onOff(processing.echoCancellation)}</dd>
        </div>
        <div>
          <dt>
            {desktop ? "Rauschfilter (WebRTC)" : "Rauschfilter (Browser)"}
          </dt>
          <dd>{onOff(processing.noiseSuppression)}</dd>
        </div>
        <div>
          <dt>{desktop ? "Auto-Gain (WebRTC)" : "Auto-Gain (Browser)"}</dt>
          <dd>{onOff(processing.autoGainControl)}</dd>
        </div>
        <div>
          <dt>Mic-Gain</dt>
          <dd>{Math.round(processing.inputGain * 100)} %</dd>
        </div>
        <div>
          <dt>Filter-Puffer</dt>
          <dd>
            {processing.addedBufferMs === null
              ? "–"
              : `${processing.addedBufferMs} ms`}
          </dd>
        </div>
      </dl>
    </section>
  );
}

function Debug() {
  const latest = useVoiceDiagnostics((state) => state.latest);
  const samples = useVoiceDiagnostics((state) => state.samples);
  const phases = useVoiceDiagnostics((state) => state.phases);
  const events = useVoiceDiagnostics((state) => state.events);
  const polling = useVoiceDiagnostics((state) => state.polling);
  const processing = useAudioProcessing();
  const flows = [
    ...(latest?.voice?.flows ?? []).map((flow) => ({ flow, watch: false })),
    ...(latest?.watch?.flows ?? []).map((flow) => ({ flow, watch: true })),
  ];
  return (
    <details className="media-disclosure">
      <summary>
        Für die Fehlersuche
        <small>Codecs, Prioritäten, Verlauf, Ereignisse</small>
      </summary>
      <div className="connection-debug">
        {latest?.caps.video ? (
          <section>
            <h4>Video auf diesem Gerät</h4>
            <VideoFacts video={latest.caps.video} />
          </section>
        ) : null}
        <section>
          <h4>Verbindungen</h4>
          <dl className="connection-facts">
            <Transport
              title="Sprache"
              snapshot={latest?.voice ?? null}
              live={polling.voice}
            />
            <Transport
              title="Watch"
              snapshot={latest?.watch ?? null}
              live={polling.watch}
            />
            <div className="is-wide">
              <dt>Audio-Graph</dt>
              <dd>{processing.contextState ?? "ohne Graph"}</dd>
            </div>
          </dl>
        </section>
        <section>
          <h4>Spuren einzeln</h4>
          {flows.length === 0 ? (
            <p>Keine RTP-Statistik.</p>
          ) : (
            <ul className="connection-flow-lines">
              {flows.map(({ flow, watch }, index) => (
                <li
                  key={`${watch}-${flow.direction}-${flow.kind}-${flow.source}-${index}`}
                >
                  <FlowLine flow={flow} watch={watch} />
                </li>
              ))}
            </ul>
          )}
          <p className="connection-debug-hint">
            Obergrenze meint die eingestellte Grenze, nicht die gemessene
            Bitrate. Prioritätswunsch: Audio hoch, Video normal. Browser und
            Netzwerk können diese Hinweise ignorieren; das garantiert keine
            Priorisierung auf dem SFU oder im Internet.
          </p>
        </section>
        <section>
          <h4>Verlauf</h4>
          {phases.length === 0 ? (
            <p>noch kein Verlauf</p>
          ) : (
            <ul className="connection-log">
              {phases.map((phase) => (
                <li key={`${phase.at}-${phase.phase}`}>
                  <span>{formatClock(phase.at)}</span>
                  <span>
                    {PHASE_LABEL[phase.phase]} · Senden{" "}
                    {formatBps(phase.audioSendBps)} · Empfang{" "}
                    {formatBps(phase.audioRecvBps)} · Video{" "}
                    {formatBps(phase.videoSendBps)}
                  </span>
                </li>
              ))}
            </ul>
          )}
          {samples.length > 0 ? (
            <div className="connection-samples">
              <table>
                <thead>
                  <tr>
                    <th scope="col">Zeit</th>
                    <th scope="col">Phase</th>
                    <th scope="col">Sprache senden</th>
                    <th scope="col">Sprache empfangen</th>
                    <th scope="col">Eigenes Video</th>
                  </tr>
                </thead>
                <tbody>
                  {samples.map((sample, index) => (
                    <tr key={`${sample.at}-${index}`}>
                      <td>{formatClock(sample.at)}</td>
                      <td>{PHASE_LABEL[sample.phase]}</td>
                      <td>{formatBps(sum(sample.voice, "audio", "send"))}</td>
                      <td>{formatBps(sum(sample.voice, "audio", "recv"))}</td>
                      <td>{formatBps(sum(sample.voice, "video", "send"))}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </section>
        <section>
          <h4>Ereignisse</h4>
          {events.length === 0 ? (
            <p>noch keine Ereignisse</p>
          ) : (
            <ul className="connection-log">
              {events.slice(-8).map((event, index) => (
                <li key={`${event.at}-${event.kind}-${event.detail}-${index}`}>
                  <span>{formatClock(event.at)}</span>
                  <span>
                    {eventLabel(event.kind)} ·{" "}
                    {event.connection === "voice" ? "Sprache" : "Watch"} ·{" "}
                    {event.detail}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </details>
  );
}

function VideoFacts({ video }: { video: VideoChoice }) {
  const encoders = video.encoders;
  return (
    <dl className="connection-facts">
      <div className="is-wide">
        <dt>Raum-Codecs</dt>
        <dd>
          {video.roomCodecs
            ? video.roomCodecs
                .map((codec) => codec.replace("video/", ""))
                .join(", ")
            : "–"}
        </dd>
      </div>
      <div>
        <dt>AV1 in Hardware</dt>
        <dd>{yesNo(encoders?.av1)}</dd>
      </div>
      <div>
        <dt>H.264 in Hardware</dt>
        <dd>{yesNo(encoders?.h264)}</dd>
      </div>
      <div>
        <dt>VP9 in Schichten</dt>
        <dd>{yesNo(encoders?.vp9Svc)}</dd>
      </div>
      <div>
        <dt>AV1 empfangen</dt>
        <dd>{yesNo(video.decodesAv1)}</dd>
      </div>
      {video.desktopApp ? (
        <div>
          <dt>Desktop-App</dt>
          <dd>v{video.desktopApp}</dd>
        </div>
      ) : null}
    </dl>
  );
}

function Transport({
  title,
  snapshot,
  live,
}: {
  title: string;
  snapshot: ConnectionSnapshot | null;
  live: boolean;
}) {
  if (!snapshot) return null;
  const { path, rttMs, availableOutgoingBps, availableIncomingBps } =
    snapshot.transport;
  return (
    <div className="is-wide">
      <dt>{title}</dt>
      <dd>
        {path === "direct" ? "direkt" : path === "turn" ? "TURN" : "–"} · RTT{" "}
        {known(formatMs(rttMs))} · Bandbreite senden{" "}
        {known(formatBps(availableOutgoingBps))}, empfangen{" "}
        {known(formatBps(availableIncomingBps))}
        {live ? "" : " · letzte Messung"}
      </dd>
    </div>
  );
}

function FlowLine({ flow, watch }: { flow: FlowStats; watch: boolean }) {
  const direction = flow.direction === "send" ? "Senden" : "Empfangen";
  return (
    <>
      <strong>
        {watch ? "Watch" : SOURCE_LABEL[flow.source]}
        {flow.kind === "video" && !watch ? " (Video)" : ""} · {direction}
      </strong>
      <span>
        Codec: {flow.codec ?? "unbekannt"} · Bitrate:{" "}
        {formatBps(flow.measuredBitrateBps)}
        {flow.configuredMaxBitrateBps !== null
          ? ` · Obergrenze: ${formatBps(flow.configuredMaxBitrateBps)}`
          : flow.direction === "send"
            ? " · keine Gelabber-Bitratengrenze"
            : ""}
      </span>
      <span>
        Paketverlust: {formatRatio(flow.packetLoss)}
        {flow.packetsLost !== null
          ? ` (${formatCount(flow.packetsLost)} Pakete)`
          : ""}{" "}
        · Jitter: {formatMs(flow.jitterMs)} · RTT: {formatMs(flow.rttMs)}
      </span>
      {flow.direction === "send" ? (
        <span>
          Senderpriorität: {flow.senderPriority ?? "nicht gemeldet"} ·
          Netzpriorität: {flow.senderNetworkPriority ?? "nicht gemeldet"}
        </span>
      ) : null}
      {flow.kind === "audio" && flow.direction === "recv" ? (
        <span>
          Ersetzte Samples:{" "}
          {flow.concealedRatio === null ||
          flow.concealedSamples === null ||
          flow.totalSamplesReceived === null
            ? "unbekannt"
            : `${formatRatio(flow.concealedRatio)} (${formatCount(flow.concealedSamples)} / ${formatCount(flow.totalSamplesReceived)})`}
        </span>
      ) : null}
      {flow.kind === "video" ? (
        <span>
          {flow.width !== null && flow.height !== null
            ? `${flow.width}×${flow.height}`
            : "Auflösung unbekannt"}{" "}
          ·{" "}
          {flow.fps !== null ? `${formatCount(flow.fps)} FPS` : "FPS unbekannt"}
          {flow.configuredMaxFps !== null
            ? ` · FPS-Obergrenze: ${flow.configuredMaxFps}`
            : ""}{" "}
          · Qualitätslimit: {formatQualityLimit(flow.qualityLimitationReason)}
        </span>
      ) : null}
    </>
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
