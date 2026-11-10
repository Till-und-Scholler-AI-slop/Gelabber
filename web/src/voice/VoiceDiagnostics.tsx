// Voice diagnostics, the settings dialog's "Diagnose" tab; the call bar's
// connection popover links here. The status on top answers whether the
// call is fine; the table shows what is being sent and received; the raw
// figures wait under "Für die Fehlersuche". They are clues, not a verdict
// about the cause.

import { Button } from "../components/ui/Button.tsx";
import { useT, type TFunction } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";
import { useAudioProcessing } from "./audioProcessing.ts";
import { ConnectionStatus } from "./ConnectionStatus.tsx";
import { flowGroups, lossTone, type FlowGroup } from "./connectionSummary.ts";
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
  unknownValue,
  useVoiceDiagnostics,
  type ConnectionSnapshot,
  type FlowStats,
  type VideoChoice,
} from "./diagnostics.ts";
import "./connection.css";
import "./quality.css";

type T = TFunction<typeof diagnostics.de>;

const onOff = (t: T, value: boolean | null | undefined) =>
  value === null || value === undefined
    ? "–"
    : value
      ? t("value.on")
      : t("value.off");

const yesNo = (t: T, value: boolean | null | undefined) =>
  value === null || value === undefined
    ? "–"
    : value
      ? t("value.yes")
      : t("value.no");

/** A figure the browser did not report reads as a dash, not "unbekannt". */
const known = (text: string) => (text === unknownValue() ? "–" : text);

export function VoiceDiagnostics() {
  const t = useT(diagnostics);
  return (
    <section aria-label={t("ui.label")} className="voice-diagnostics">
      <ConnectionStatus>
        <UploadTip />
      </ConnectionStatus>
      <Flows />
      <Microphone />
      <Debug />
      <footer className="connection-footer">
        <Button
          size="sm"
          variant="ghost"
          onClick={downloadDiagnostics}
          title={t("ui.downloadTitle")}
        >
          {t("ui.download")}
        </Button>
      </footer>
    </section>
  );
}

/** Loss on the way out of a weak call: the economy mode is the setting
 * that helps, and it sits on the video tab. */
function UploadTip() {
  const t = useT(diagnostics);
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
      <p>{t("tip.text")}</p>
      <Button size="sm" onClick={() => openDialog("video")}>
        {t("tip.action")}
      </Button>
    </div>
  );
}

function Flows() {
  const t = useT(diagnostics);
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
    audioCap === null
      ? null
      : t("flows.limitVoice", { bitrate: formatBps(audioCap) }),
    videoCap === null
      ? null
      : t("flows.limitVideo", { bitrate: formatBps(videoCap) }),
  ].filter((limit) => limit !== null);
  return (
    <section aria-labelledby="connection-flows-title">
      <div className="connection-block-head">
        <h3 id="connection-flows-title">{t("flows.title")}</h3>
        {groups.length > 0 && !measuring ? (
          <span>{t("flows.lastMeasurement")}</span>
        ) : null}
      </div>
      {groups.length === 0 ? (
        <p className="connection-empty">
          {measuring ? t("flows.waiting") : t("flows.idle")}
        </p>
      ) : (
        <div className="connection-flows-box">
          <table className="connection-flows">
            <thead>
              <tr>
                <th scope="col">{t("flows.track")}</th>
                <th scope="col" className="is-codec">
                  {t("flows.codec")}
                </th>
                <th scope="col" className="is-number">
                  {t("flows.bitrate")}
                </th>
                <th scope="col" className="is-number">
                  {t("flows.loss")}
                </th>
                <th scope="col" className="is-number">
                  {t("flows.jitter")}
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
          ? t("flows.noLimit")
          : t("flows.limits", {
              count: limits.length,
              limits: limits.join(", "),
            })}{" "}
        {t("flows.maxFps", { fps: VIDEO_MAX_FPS })}
      </p>
    </section>
  );
}

function FlowRow({ group }: { group: FlowGroup }) {
  const t = useT(diagnostics);
  const send = group.direction === "send";
  const count =
    group.count < 2
      ? null
      : send && group.kind === "video"
        ? t("flows.layers", { count: group.count })
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
        <span className="sr-only">
          {send ? t("flows.sending") : t("flows.receiving")}{" "}
        </span>
        {group.name}
        {count ? <small> {count}</small> : null}
        {picture.length > 0 || group.limitedBy ? (
          <span className="connection-flow-sub">
            {picture.length > 0 ? <span>{picture.join(" · ")}</span> : null}
            {group.limitedBy ? (
              <span className="connection-chip">
                {t("flows.limitedBy", {
                  reason: formatQualityLimit(group.limitedBy),
                })}
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
  const t = useT(diagnostics);
  const processing = useAudioProcessing();
  const format = [
    processing.sampleRate === null
      ? null
      : t("mic.kiloHertz", { rate: processing.sampleRate / 1000 }),
    processing.channels === null
      ? null
      : processing.channels === 1
        ? t("mic.mono")
        : processing.channels === 2
          ? t("mic.stereo")
          : t("mic.channels", { count: processing.channels }),
  ].filter(Boolean);
  return (
    <section aria-labelledby="connection-mic-title">
      <div className="connection-block-head">
        <h3 id="connection-mic-title">{t("mic.title")}</h3>
      </div>
      <dl className="connection-facts">
        <div className="is-wide">
          <dt>{t("mic.processing")}</dt>
          <dd>{processing.message}</dd>
        </div>
        <div>
          <dt>{t("mic.format")}</dt>
          <dd>{format.length > 0 ? format.join(", ") : "–"}</dd>
        </div>
        <div>
          <dt>{t("mic.echo")}</dt>
          <dd>{onOff(t, processing.echoCancellation)}</dd>
        </div>
        <div>
          <dt>{t("mic.noise")}</dt>
          <dd>{onOff(t, processing.noiseSuppression)}</dd>
        </div>
        <div>
          <dt>{t("mic.autoGain")}</dt>
          <dd>{onOff(t, processing.autoGainControl)}</dd>
        </div>
        <div>
          <dt>{t("mic.gain")}</dt>
          <dd>
            {t("percent", { percent: Math.round(processing.inputGain * 100) })}
          </dd>
        </div>
        <div>
          <dt>{t("mic.buffer")}</dt>
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
  const t = useT(diagnostics);
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
        {t("debug.title")}
        <small>{t("debug.subtitle")}</small>
      </summary>
      <div className="connection-debug">
        {latest?.caps.video ? (
          <section>
            <h4>{t("debug.video")}</h4>
            <VideoFacts video={latest.caps.video} />
          </section>
        ) : null}
        <section>
          <h4>{t("debug.connections")}</h4>
          <dl className="connection-facts">
            <Transport
              title={t("source.voice")}
              snapshot={latest?.voice ?? null}
              live={polling.voice}
            />
            <Transport
              title={t("source.watch")}
              snapshot={latest?.watch ?? null}
              live={polling.watch}
            />
            <div className="is-wide">
              <dt>{t("debug.audioGraph")}</dt>
              <dd>{processing.contextState ?? t("debug.noGraph")}</dd>
            </div>
          </dl>
        </section>
        <section>
          <h4>{t("debug.tracks")}</h4>
          {flows.length === 0 ? (
            <p>{t("debug.noRtp")}</p>
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
          <p className="connection-debug-hint">{t("debug.hint")}</p>
        </section>
        <section>
          <h4>{t("debug.history")}</h4>
          {phases.length === 0 ? (
            <p>{t("debug.noHistory")}</p>
          ) : (
            <ul className="connection-log">
              {phases.map((phase) => (
                <li key={`${phase.at}-${phase.phase}`}>
                  <span>{formatClock(phase.at)}</span>
                  <span>
                    {t("debug.phaseLine", {
                      phase: t(PHASE_LABEL[phase.phase]),
                      send: formatBps(phase.audioSendBps),
                      recv: formatBps(phase.audioRecvBps),
                      video: formatBps(phase.videoSendBps),
                    })}
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
                    <th scope="col">{t("debug.time")}</th>
                    <th scope="col">{t("debug.phase")}</th>
                    <th scope="col">{t("debug.voiceSent")}</th>
                    <th scope="col">{t("debug.voiceReceived")}</th>
                    <th scope="col">{t("debug.ownVideo")}</th>
                  </tr>
                </thead>
                <tbody>
                  {samples.map((sample, index) => (
                    <tr key={`${sample.at}-${index}`}>
                      <td>{formatClock(sample.at)}</td>
                      <td>{t(PHASE_LABEL[sample.phase])}</td>
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
          <h4>{t("debug.events")}</h4>
          {events.length === 0 ? (
            <p>{t("debug.noEvents")}</p>
          ) : (
            <ul className="connection-log">
              {events.slice(-8).map((event, index) => (
                <li key={`${event.at}-${event.kind}-${event.detail}-${index}`}>
                  <span>{formatClock(event.at)}</span>
                  <span>
                    {eventLabel(t, event.kind)} ·{" "}
                    {event.connection === "voice"
                      ? t("source.voice")
                      : t("source.watch")}{" "}
                    · {event.detail}
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
  const t = useT(diagnostics);
  const encoders = video.encoders;
  return (
    <dl className="connection-facts">
      <div className="is-wide">
        <dt>{t("debug.roomCodecs")}</dt>
        <dd>
          {video.roomCodecs
            ? video.roomCodecs
                .map((codec) => codec.replace("video/", ""))
                .join(", ")
            : "–"}
        </dd>
      </div>
      <div>
        <dt>{t("debug.av1Hardware")}</dt>
        <dd>{yesNo(t, encoders?.av1)}</dd>
      </div>
      <div>
        <dt>{t("debug.h264Hardware")}</dt>
        <dd>{yesNo(t, encoders?.h264)}</dd>
      </div>
      <div>
        <dt>{t("debug.vp9Layers")}</dt>
        <dd>{yesNo(t, encoders?.vp9Svc)}</dd>
      </div>
      <div>
        <dt>{t("debug.av1Receive")}</dt>
        <dd>{yesNo(t, video.decodesAv1)}</dd>
      </div>
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
  const t = useT(diagnostics);
  if (!snapshot) return null;
  const { path, rttMs, availableOutgoingBps, availableIncomingBps } =
    snapshot.transport;
  return (
    <div className="is-wide">
      <dt>{title}</dt>
      <dd>
        {t("transport.line", {
          path:
            path === "direct"
              ? t("transport.direct")
              : path === "turn"
                ? "TURN"
                : "–",
          rtt: known(formatMs(rttMs)),
          up: known(formatBps(availableOutgoingBps)),
          down: known(formatBps(availableIncomingBps)),
        })}
        {live ? "" : ` · ${t("flows.lastMeasurement")}`}
      </dd>
    </div>
  );
}

function FlowLine({ flow, watch }: { flow: FlowStats; watch: boolean }) {
  const t = useT(diagnostics);
  const direction =
    flow.direction === "send" ? t("line.send") : t("line.receive");
  const source = watch ? t("source.watch") : t(SOURCE_LABEL[flow.source]);
  const dot = (parts: (string | null)[]) =>
    parts.filter((part) => part !== null).join(" · ");
  const notReported = t("line.notReported");
  return (
    <>
      <strong>
        {flow.kind === "video" && !watch
          ? t("line.video", { name: source })
          : source}{" "}
        · {direction}
      </strong>
      <span>
        {dot([
          t("line.codec", { codec: flow.codec ?? unknownValue() }),
          t("line.bitrate", { bitrate: formatBps(flow.measuredBitrateBps) }),
          flow.configuredMaxBitrateBps !== null
            ? t("line.cap", { cap: formatBps(flow.configuredMaxBitrateBps) })
            : flow.direction === "send"
              ? t("line.noCap")
              : null,
        ])}
      </span>
      <span>
        {dot([
          flow.packetsLost !== null
            ? t("line.lossPackets", {
                loss: formatRatio(flow.packetLoss),
                packets: formatCount(flow.packetsLost),
              })
            : t("line.loss", { loss: formatRatio(flow.packetLoss) }),
          t("line.jitter", { jitter: formatMs(flow.jitterMs) }),
          t("line.rtt", { rtt: formatMs(flow.rttMs) }),
        ])}
      </span>
      {flow.direction === "send" ? (
        <span>
          {dot([
            t("line.senderPriority", {
              priority: flow.senderPriority ?? notReported,
            }),
            t("line.networkPriority", {
              priority: flow.senderNetworkPriority ?? notReported,
            }),
          ])}
        </span>
      ) : null}
      {flow.kind === "audio" && flow.direction === "recv" ? (
        <span>
          {t("line.concealed", {
            value:
              flow.concealedRatio === null ||
              flow.concealedSamples === null ||
              flow.totalSamplesReceived === null
                ? unknownValue()
                : `${formatRatio(flow.concealedRatio)} (${formatCount(flow.concealedSamples)} / ${formatCount(flow.totalSamplesReceived)})`,
          })}
        </span>
      ) : null}
      {flow.kind === "video" ? (
        <span>
          {dot([
            flow.width !== null && flow.height !== null
              ? `${flow.width}×${flow.height}`
              : t("line.resolutionUnknown"),
            flow.fps !== null
              ? `${formatCount(flow.fps)} FPS`
              : t("line.fpsUnknown"),
            flow.configuredMaxFps !== null
              ? t("line.fpsCap", { fps: flow.configuredMaxFps })
              : null,
            t("line.qualityLimit", {
              limit: formatQualityLimit(flow.qualityLimitationReason),
            }),
          ])}
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

function eventLabel(t: T, kind: string): string {
  switch (kind) {
    case "stream-start":
      return t("event.streamStart");
    case "stream-stop":
      return t("event.streamStop");
    case "device-change":
      return t("event.deviceChange");
    case "sdp-error":
      return t("event.sdpError");
    case "ice-error":
      return t("event.iceError");
    case "recovery":
      return t("event.recovery");
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
