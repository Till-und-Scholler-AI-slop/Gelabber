import { useEffect, useId, useRef, useState } from "react";
import { MicIcon } from "../components/Icons.tsx";
import { Modal } from "../components/Modal.tsx";
import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import {
  createMicrophoneComparison,
  type ComparisonState,
  type ComparisonClips,
} from "./microphoneComparison.ts";
import {
  AUDIO_PROCESSING,
  useMediaSettings,
  type AudioProcessingMode,
} from "./settings.ts";
import "./room.css";

export function MicrophoneTest({ onClose }: { onClose: () => void }) {
  const t = useT(voiceSettings);
  const tc = useT(common);
  const session = useRef<ReturnType<typeof createMicrophoneComparison> | null>(
    null,
  );
  const id = useId();
  const [state, setState] = useState<ComparisonState>(() => ({
    phase: "idle",
    message: t("micTest.intro"),
  }));
  const [levels, setLevels] = useState({
    before: 0,
    after: 0,
    clipping: false,
  });
  const [clips, setClips] = useState<ComparisonClips | null>(null);
  const [recording, setRecording] = useState(false);
  const processingMode = useMediaSettings((s) => s.processingMode);
  const patch = useMediaSettings((s) => s.patch);
  const modes = AUDIO_PROCESSING;
  const running = state.phase === "active" || state.phase === "pending";
  useEffect(() => () => session.current?.stop(), []);
  function start() {
    session.current?.stop();
    session.current = createMicrophoneComparison(useMediaSettings.getState(), {
      state: setState,
      levels: setLevels,
      clips: setClips,
      recording: setRecording,
    });
    void session.current.start();
  }
  function stop() {
    session.current?.stop();
    setLevels({ before: 0, after: 0, clipping: false });
    setState({
      phase: "idle",
      message: t("micTest.stopped"),
    });
  }
  function close() {
    session.current?.stop();
    onClose();
  }
  return (
    <Modal open onClose={close} title={t("micTest.title")}>
      <div className="voice-microphone-test">
        <MicIcon size={36} />
        <p className="voice-room-muted">{t("micTest.local")}</p>
        <label htmlFor={`${id}-before`}>{t("micTest.inputLevel")}</label>
        <meter id={`${id}-before`} min={0} max={100} value={levels.before}>
          {levels.before}%
        </meter>
        <label htmlFor={`${id}-after`}>{t("micTest.processed")}</label>
        <meter id={`${id}-after`} min={0} max={100} value={levels.after}>
          {levels.after}%
        </meter>
        <fieldset className="voice-microphone-test-modes">
          <legend>{t("micTest.processing")}</legend>
          {(Object.keys(modes) as AudioProcessingMode[]).map((mode) => (
            <label key={mode}>
              <input
                type="radio"
                className="ui-check"
                name={`${id}-processing`}
                checked={processingMode === mode}
                onChange={() => {
                  patch({ processingMode: mode });
                  // A running test compares with the new mode straight away.
                  if (running) start();
                }}
              />
              {t(modes[mode].label)}
            </label>
          ))}
        </fieldset>
        <p role="status">{state.message}</p>
        {levels.clipping && <p role="status">{t("micTest.clipping")}</p>}
        <div className="voice-microphone-test-actions">
          {running ? (
            <Button onClick={stop}>{t("micTest.stop")}</Button>
          ) : (
            <Button variant="primary" onClick={start}>
              {t("micTest.start")}
            </Button>
          )}
          {state.phase === "active" && (
            <Button
              onClick={() =>
                recording
                  ? session.current?.endRecording()
                  : session.current?.record()
              }
            >
              {recording
                ? t("micTest.stopRecording")
                : t("micTest.startRecording")}
            </Button>
          )}
          <Button variant="ghost" onClick={close}>
            {tc("close")}
          </Button>
        </div>
        {recording && <p role="status">{t("micTest.recording")}</p>}
        {clips && (
          <section
            className="flex w-full flex-col gap-3"
            aria-label={t("micTest.comparison")}
          >
            <label>
              {t("micTest.inputClip")}
              <audio controls src={clips.before} className="w-full" />
            </label>
            <label>
              {t("micTest.processed")}
              <audio controls src={clips.after} className="w-full" />
            </label>
            <Button
              size="sm"
              variant="ghost"
              className="self-start"
              onClick={() => session.current?.clearClips()}
            >
              {t("micTest.deleteClips")}
            </Button>
          </section>
        )}
        <p className="voice-room-muted">{t("micTest.switchNote")}</p>
      </div>
    </Modal>
  );
}
