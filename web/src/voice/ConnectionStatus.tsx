// How the call's connection is doing at a glance: the call bar's bars, one
// plain sentence, ping, loss and path, and the ping of the last samples.
// The diagnostics tab draws it in full, the call bar's popover compact.

import type { ReactNode } from "react";

import { useT } from "../i18n/catalog.ts";
import { diagnostics } from "../i18n/catalogs/diagnostics.ts";
import { formatNumber } from "../i18n/format.ts";

import {
  lossTone,
  pingHistory,
  pingTone,
  type QualityTone,
} from "./connectionSummary.ts";
import {
  DIAGNOSTIC_LIMITS,
  callQuality,
  callQualityDetail,
  useVoiceDiagnostics,
} from "./diagnostics.ts";
import "./connection.css";

const PATH_LABEL = {
  direct: "status.pathDirect",
  turn: "status.pathRelay",
} as const;

function percent(ratio: number): string {
  return formatNumber(ratio * 100, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  });
}

/** The call bar's four bars, larger; level 0 is not measured yet. */
export function SignalBars({ level }: { level: number }) {
  return (
    <span className="connection-signal" data-level={level} aria-hidden="true">
      {[1, 2, 3, 4].map((bar) => (
        <span key={bar} />
      ))}
    </span>
  );
}

export function ConnectionStatus({
  compact = false,
  children,
}: {
  compact?: boolean;
  /** Drawn at the end of the block, such as a tip for a weak call. */
  children?: ReactNode;
}) {
  const t = useT(diagnostics);
  const sample = useVoiceDiagnostics((state) => state.latest?.voice ?? null);
  const samples = useVoiceDiagnostics((state) => state.samples);
  const measuring = useVoiceDiagnostics((state) => state.polling.voice);
  const quality = measuring ? callQuality(sample) : null;
  const detail = quality ? callQualityDetail(quality) : null;
  const count = compact ? 30 : 45;
  const path = quality?.path ? t(PATH_LABEL[quality.path]) : null;
  return (
    <section
      className={`connection-status${compact ? " is-compact" : ""}`}
      data-level={quality?.level ?? 0}
      aria-label={t("status.label")}
    >
      <div className="connection-status-head">
        <SignalBars level={quality?.level ?? 0} />
        <div className="connection-status-verdict">
          <h3>
            {quality
              ? t("status.verdict", { label: quality.label })
              : measuring
                ? t("status.measuring")
                : t("status.notMeasuring")}
          </h3>
          {!compact && detail ? <p>{detail}</p> : null}
        </div>
        {compact ? (
          quality?.path ? (
            <span className="connection-status-meta">
              {quality.path === "direct"
                ? t("status.direct")
                : t("status.relay")}
            </span>
          ) : null
        ) : measuring ? (
          <span className="connection-status-meta is-live">
            {t("status.interval", {
              seconds: DIAGNOSTIC_LIMITS.intervalMs / 1000,
            })}
          </span>
        ) : null}
      </div>
      <dl className="connection-figures">
        <Figure
          label={t("status.ping")}
          value={quality?.rttMs ?? null}
          format={(ms) => String(Math.round(ms))}
          unit="ms"
          tone={pingTone}
        />
        <Figure
          label={t("status.packetLoss")}
          value={quality?.loss ?? null}
          format={percent}
          unit="%"
          tone={lossTone}
        />
        {compact ? null : (
          <div>
            <dt>{t("status.connected")}</dt>
            <dd>{path ?? "–"}</dd>
          </div>
        )}
      </dl>
      <PingTrace
        history={pingHistory(samples, count)}
        seconds={(count * DIAGNOSTIC_LIMITS.intervalMs) / 1000}
      />
      {children}
    </section>
  );
}

function Figure({
  label,
  value,
  format,
  unit,
  tone,
}: {
  label: string;
  value: number | null;
  format: (value: number) => string;
  unit: string;
  tone: (value: number) => QualityTone;
}) {
  return (
    <div data-tone={value === null ? undefined : tone(value)}>
      <dt>{label}</dt>
      <dd>
        {value === null ? (
          "–"
        ) : (
          <>
            {format(value)}
            <small> {unit}</small>
          </>
        )}
      </dd>
    </div>
  );
}

/** One bar per sample, newest on the right; taller is slower. */
function PingTrace({
  history,
  seconds,
}: {
  history: (number | null)[];
  seconds: number;
}) {
  const t = useT(diagnostics);
  const known = history.filter((ms): ms is number => ms !== null);
  const low = Math.round(Math.min(...known));
  const high = Math.round(Math.max(...known));
  // Below 100 ms the bars stay low: a calm line reads as a calm call.
  const top = Math.max(100, ...known);
  return (
    <div className="connection-trace">
      <div className="connection-trace-caption" aria-hidden="true">
        <span>{t("trace.caption", { seconds })}</span>
        {known.length > 0 ? (
          <span>{low === high ? `${low} ms` : `${low}–${high} ms`}</span>
        ) : null}
      </div>
      <div
        className="connection-trace-bars"
        role="img"
        aria-label={
          known.length > 0
            ? t("trace.label", {
                seconds,
                low: String(low),
                high: String(high),
              })
            : t("trace.labelEmpty", { seconds })
        }
      >
        {history.map((ms, index) => (
          <span
            key={index}
            data-tone={ms === null ? undefined : pingTone(ms)}
            style={{
              height: ms === null ? 0 : `${Math.max(8, (ms / top) * 100)}%`,
            }}
          />
        ))}
      </div>
      <div className="connection-trace-axis" aria-hidden="true">
        <span>{t("trace.ago", { seconds })}</span>
        <span>{t("trace.now")}</span>
      </div>
    </div>
  );
}
