import { useEffect, useId, useRef, useState } from "react";

import { MicIcon } from "../components/Icons.tsx";
import { Modal } from "../components/Modal.tsx";
import {
  createMicrophoneSession,
  type MicrophoneTestState,
} from "./microphone.ts";
import { micConstraints } from "./settings.ts";
import "./room.css";

export function MicrophoneTest({ onClose }: { onClose: () => void }) {
  const session = useRef<ReturnType<typeof createMicrophoneSession> | null>(
    null,
  );
  const id = useId();
  const [state, setState] = useState<MicrophoneTestState>({
    phase: "idle",
    message: "Prüfe in Ruhe, ob dein Mikrofon bereit ist.",
  });
  const [level, setLevel] = useState(0);
  useEffect(() => () => session.current?.stop(), []);

  function start() {
    session.current?.stop();
    setLevel(0);
    session.current = createMicrophoneSession(
      { onState: setState, onLevel: setLevel },
      undefined,
      micConstraints(),
    );
    void session.current.start();
  }
  function stop() {
    session.current?.stop();
    setLevel(0);
    setState({
      phase: "idle",
      message: "Test beendet. Dein Mikrofon ist wieder ausgeschaltet.",
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
          Der Test läuft lokal. Es wird nichts aufgezeichnet, abgespielt oder
          übertragen.
        </p>
        <label htmlFor={id}>Mikrofonpegel</label>
        <meter id={id} min={0} max={100} value={level}>
          {level}%
        </meter>
        <span className="voice-room-muted">
          {state.phase === "active" ? `${level}%` : "Noch kein Signal"}
        </span>
        <p role="status">{state.message}</p>
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
          <button
            type="button"
            className="voice-room-text-button"
            onClick={close}
          >
            Schließen
          </button>
        </div>
        <p className="voice-room-muted">
          Beim Schließen wird der Mikrofonzugriff beendet. Deine
          Voice-Einstellungen bleiben erhalten.
        </p>
      </div>
    </Modal>
  );
}
