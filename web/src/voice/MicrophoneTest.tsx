import { useEffect, useId, useRef, useState } from "react";
import { MicIcon } from "../components/Icons.tsx";
import { Modal } from "../components/Modal.tsx";
import { Button } from "../components/ui/Button.tsx";
import {
  createMicrophoneComparison,
  type ComparisonState,
  type ComparisonClips,
} from "./microphoneComparison.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { createNativeMicrophoneTest } from "./native/microphoneTest.ts";
import {
  AUDIO_PROCESSING,
  AUDIO_PROCESSING_DESKTOP,
  useMediaSettings,
  type AudioProcessingMode,
} from "./settings.ts";
import "./room.css";

export function MicrophoneTest({ onClose }: { onClose: () => void }) {
  const session = useRef<ReturnType<typeof createMicrophoneComparison> | null>(
    null,
  );
  const id = useId();
  const [state, setState] = useState<ComparisonState>({
    phase: "idle",
    message: "Prüfe dein Mikrofon und vergleiche die Verarbeitung.",
  });
  const [levels, setLevels] = useState({
    before: 0,
    after: 0,
    clipping: false,
  });
  const [clips, setClips] = useState<ComparisonClips | null>(null);
  const [recording, setRecording] = useState(false);
  const processingMode = useMediaSettings((s) => s.processingMode);
  const patch = useMediaSettings((s) => s.patch);
  const modes = isDesktopApp() ? AUDIO_PROCESSING_DESKTOP : AUDIO_PROCESSING;
  const running = state.phase === "active" || state.phase === "pending";
  useEffect(() => () => session.current?.stop(), []);
  function start() {
    session.current?.stop();
    const create = isDesktopApp()
      ? createNativeMicrophoneTest
      : createMicrophoneComparison;
    session.current = create(useMediaSettings.getState(), {
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
      message: "Test beendet. Mikrofon und Aufnahmen wurden freigegeben.",
    });
  }
  function close() {
    session.current?.stop();
    onClose();
  }
  return (
    <Modal open onClose={close} title="Mikrofon testen">
      <div className="voice-microphone-test">
        <MicIcon size={36} />
        <p>Nur du und dein Mikrofon.</p>
        <p className="voice-room-muted">
          {isDesktopApp()
            ? "Der Test bleibt lokal. Es wird nichts aufgenommen oder übertragen."
            : "Der Test bleibt lokal. Eine Aufnahme startet erst mit deinem Klick, dauert höchstens acht Sekunden und wird beim Löschen oder Schließen entfernt. Es wird nichts übertragen."}
        </p>
        <label htmlFor={`${id}-before`}>Eingangspegel</label>
        <meter id={`${id}-before`} min={0} max={100} value={levels.before}>
          {levels.before}%
        </meter>
        <label htmlFor={`${id}-after`}>Nach Verarbeitung</label>
        <meter id={`${id}-after`} min={0} max={100} value={levels.after}>
          {levels.after}%
        </meter>
        <fieldset className="voice-microphone-test-modes">
          <legend>Verarbeitung</legend>
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
              {modes[mode].label}
            </label>
          ))}
        </fieldset>
        <p role="status">{state.message}</p>
        {levels.clipping && (
          <p role="status">
            Übersteuerung erkannt. Verringere den Mic-Gain oder Gerätepegel.
          </p>
        )}
        <div className="voice-microphone-test-actions">
          {running ? (
            <Button onClick={stop}>Test beenden</Button>
          ) : (
            <Button variant="primary" onClick={start}>
              Test starten
            </Button>
          )}
          {state.phase === "active" && !isDesktopApp() && (
            <Button
              onClick={() =>
                recording
                  ? session.current?.endRecording()
                  : session.current?.record()
              }
            >
              {recording ? "Aufnahme beenden" : "A/B-Aufnahme starten"}
            </Button>
          )}
          <Button variant="ghost" onClick={close}>
            Schließen
          </Button>
        </div>
        {recording && (
          <p role="status">Lokale Aufnahme läuft (maximal acht Sekunden).</p>
        )}
        {clips && (
          <section
            className="flex w-full flex-col gap-3"
            aria-label="Lokaler Audiovergleich"
          >
            <label>
              Eingang (einschließlich Browser-Capture)
              <audio controls src={clips.before} className="w-full" />
            </label>
            <label>
              Nach Verarbeitung
              <audio controls src={clips.after} className="w-full" />
            </label>
            <Button
              size="sm"
              variant="ghost"
              className="self-start"
              onClick={() => session.current?.clearClips()}
            >
              Aufnahmen löschen
            </Button>
          </section>
        )}
        <p className="voice-room-muted">
          {isDesktopApp()
            ? "Wechselst du die Verarbeitung, misst der Test gleich mit ihr weiter."
            : "Vorhandene Browserfilter wirken bereits auf den Eingang. Wechselst du die Verarbeitung, startet der Vergleich gleich mit ihr neu; Aufnahmen werden dabei gelöscht."}
        </p>
      </div>
    </Modal>
  );
}
