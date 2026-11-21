// Input sensitivity (GEL-27): the gate's switch between automatic and a
// threshold, and a level meter that shows where the threshold sits. The
// meter follows the call's microphone, or a capture of its own on demand.

import { useEffect, useState, type CSSProperties } from "react";

import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import { formatNumber } from "../i18n/format.ts";
import {
  captureMicrophone,
  useInputGate,
  type GateReport,
  type MicProcessor,
} from "./audioProcessing.ts";
import {
  GATE_THRESHOLD_MAX,
  GATE_THRESHOLD_MIN,
  useMediaSettings,
} from "./settings.ts";
import { SettingRow, Slider, SwitchLine } from "./settingsLayout.tsx";
import { useVoice } from "./voiceStore.ts";

/** A report older than this belongs to a gate that stopped. */
const REPORT_STALE_MS = 600;

/** Where `db` sits on the meter and the slider, in percent. */
function meterPosition(db: number): number {
  const range = GATE_THRESHOLD_MAX - GATE_THRESHOLD_MIN;
  return (
    (Math.min(GATE_THRESHOLD_MAX, Math.max(GATE_THRESHOLD_MIN, db)) -
      GATE_THRESHOLD_MIN) *
    (100 / range)
  );
}

/** The latest report while one keeps coming, otherwise null. */
function useLiveReport(): GateReport | null {
  const { report, at } = useInputGate();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!report) return;
    const timer = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(timer);
  }, [report]);
  return report && now - at < REPORT_STALE_MS + 250 ? report : null;
}

/** A capture for the meter only: nothing is sent or played. */
function useLevelCheck(on: boolean): boolean {
  const device = useMediaSettings((s) => s.audioInputId);
  const mode = useMediaSettings((s) => s.processingMode);
  const echo = useMediaSettings((s) => s.echoCancellation);
  const noise = useMediaSettings((s) => s.noiseSuppression);
  const autoGain = useMediaSettings((s) => s.autoGainControl);
  // The capture that failed; a new one starts without the failure.
  const attempt = [on, device, mode, echo, noise, autoGain].join("|");
  const [failedAttempt, setFailedAttempt] = useState<string | null>(null);
  useEffect(() => {
    if (!on) return;
    let current = true;
    let raw: MediaStream | null = null;
    let processor: MicProcessor | null = null;
    const setFailed = () => {
      if (current) setFailedAttempt(attempt);
    };
    const unsubscribe = useMediaSettings.subscribe((next, prev) => {
      if (next.inputGain !== prev.inputGain) processor?.setGain(next.inputGain);
      processor?.setGate(next);
    });
    void captureMicrophone(
      (constraints) => navigator.mediaDevices.getUserMedia(constraints),
      useMediaSettings.getState(),
      false,
      setFailed,
      {
        current: () => current,
        acquired() {},
        discarded() {},
      },
    ).then((result) => {
      raw = result.raw;
      processor = result.processor;
      if (!current) stop();
    }, setFailed);
    function stop() {
      processor?.dispose();
      raw?.getTracks().forEach((track) => track.stop());
    }
    return () => {
      current = false;
      unsubscribe();
      stop();
    };
  }, [attempt, on, device, mode, echo, noise, autoGain]);
  return on && failedAttempt === attempt;
}

export function InputSensitivity() {
  const t = useT(voiceSettings);
  const gateAuto = useMediaSettings((s) => s.gateAuto);
  const threshold = useMediaSettings((s) => s.gateThresholdDb);
  const voiceInput = useMediaSettings((s) => s.voiceInput);
  const mode = useMediaSettings((s) => s.processingMode);
  const patch = useMediaSettings((s) => s.patch);
  const joined = useVoice((s) => s.status === "joined");
  const [checking, setChecking] = useState(false);
  const off = voiceInput === "ptt" || mode === "original";
  const failed = useLevelCheck(checking && !joined && !off);
  const report = useLiveReport();
  const db = (value: number) =>
    t("gate.db", { db: formatNumber(Math.round(value)) });

  if (off)
    return (
      <SettingRow label={t("gate.title")}>
        <p className="media-hint media-row-text">
          {voiceInput === "ptt" ? t("gate.pttOff") : t("gate.originalOff")}
        </p>
      </SettingRow>
    );

  const live = report && report.mode !== "off" ? report : null;
  const shownThreshold = gateAuto ? (live?.thresholdDb ?? null) : threshold;
  return (
    <SettingRow label={t("gate.title")} htmlFor="gate-auto">
      <SwitchLine
        id="gate-auto"
        checked={gateAuto}
        onChange={(value) => patch({ gateAuto: value })}
      >
        {t("gate.autoHint")}
      </SwitchLine>
      <div className="media-gate">
        <div
          className="media-gate-meter"
          role="meter"
          aria-label={t("gate.meter")}
          aria-valuemin={GATE_THRESHOLD_MIN}
          aria-valuemax={GATE_THRESHOLD_MAX}
          aria-valuenow={live ? Math.round(live.levelDb) : GATE_THRESHOLD_MIN}
          aria-valuetext={
            live
              ? `${db(live.levelDb)}, ${t(live.open ? "gate.open" : "gate.closed")}`
              : t("gate.idle")
          }
          data-open={live?.open ? "true" : "false"}
          style={
            {
              "--gate-level": `${live ? meterPosition(live.levelDb) : 0}%`,
              "--gate-threshold": `${shownThreshold === null ? 0 : meterPosition(shownThreshold)}%`,
            } as CSSProperties
          }
        >
          <span className="media-gate-level" />
          {shownThreshold !== null && <span className="media-gate-threshold" />}
        </div>
        <output className="media-gate-value">
          {live ? db(live.levelDb) : ""}
        </output>
        <p role="status" className="media-hint media-gate-state">
          {live
            ? `${t(live.open ? "gate.open" : "gate.closed")} · ${
                gateAuto
                  ? t("gate.autoAt", {
                      db: formatNumber(Math.round(live.thresholdDb)),
                    })
                  : db(threshold)
              }`
            : failed
              ? t("gate.checkFailed")
              : t("gate.idle")}
        </p>
      </div>
      {!gateAuto && (
        <label htmlFor="gate-threshold" className="media-hint">
          {t("gate.threshold")}
        </label>
      )}
      {!gateAuto && (
        <Slider
          id="gate-threshold"
          min={GATE_THRESHOLD_MIN}
          max={GATE_THRESHOLD_MAX}
          value={threshold}
          suffix={db(threshold)}
          onChange={(value) => patch({ gateThresholdDb: value })}
          hint={t("gate.thresholdHint")}
        />
      )}
      {!joined && (
        <div className="media-actions">
          <Button size="sm" onClick={() => setChecking((on) => !on)}>
            {checking ? t("gate.stopCheck") : t("gate.check")}
          </Button>
        </div>
      )}
    </SettingRow>
  );
}
