import { useEffect, useId, useRef, useState } from "react";
import { MicIcon } from "../components/Icons.tsx";
import { Modal } from "../components/Modal.tsx";
import {
  createMicrophoneComparison,
  type ComparisonState,
  type ComparisonClips,
} from "./microphoneComparison.ts";
import { useMediaSettings } from "./settings.ts";
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
          Der Test bleibt lokal. Eine Aufnahme startet erst mit deinem Klick,
          dauert höchstens acht Sekunden und wird beim Löschen oder Schließen
          entfernt. Es wird nichts übertragen.
        </p>
        <label htmlFor={`${id}-before`}>Eingangspegel</label>
        <meter id={`${id}-before`} min={0} max={100} value={levels.before}>
          {levels.before}%
        </meter>
        <label htmlFor={`${id}-after`}>Nach Verarbeitung</label>
        <meter id={`${id}-after`} min={0} max={100} value={levels.after}>
          {levels.after}%
        </meter>
        <p role="status">{state.message}</p>
        {levels.clipping && (
          <p role="status">
            Übersteuerung erkannt. Verringere den Mic-Gain oder Gerätepegel.
          </p>
        )}
        <div className="voice-room-actions">
          {state.phase === "active" || state.phase === "pending" ? (
            <button type="button" className="voice-room-button" onClick={stop}>
              Test beenden
            </button>
          ) : (
            <button
              type="button"
              className="voice-room-button voice-room-primary"
              onClick={start}
            >
              Test starten
            </button>
          )}
          {state.phase === "active" && (
            <button
              type="button"
              className="voice-room-button"
              onClick={() =>
                recording
                  ? session.current?.endRecording()
                  : session.current?.record()
              }
            >
              {recording ? "Aufnahme beenden" : "A/B-Aufnahme starten"}
            </button>
          )}
          <button
            type="button"
            className="voice-room-text-button"
            onClick={close}
          >
            Schließen
          </button>
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
            <button
              type="button"
              className="voice-room-text-button"
              onClick={() => session.current?.clearClips()}
            >
              Aufnahmen löschen
            </button>
          </section>
        )}
        <p className="voice-room-muted">
          Vorhandene Browserfilter wirken bereits auf den Eingang. Für einen
          Vergleich der Modi beende den Test, ändere die Einstellung und starte
          erneut.
        </p>
      </div>
    </Modal>
  );
}
