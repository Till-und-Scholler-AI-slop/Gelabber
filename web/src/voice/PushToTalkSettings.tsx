// Voice activity or push-to-talk, the key and its release delay (pushToTalk.ts).

import { useEffect, useState } from "react";
import { isDesktopApp } from "./native/bridge.ts";
import {
  globalKeyNote,
  onWindows,
  PTT_RELEASE_DELAY_MAX,
  pushToTalkCode,
  pushToTalkKeyLabel,
  pushToTalkMouseCode,
  usePushToTalk,
  type VoiceInputMode,
} from "./pushToTalk.ts";
import { useMediaSettings } from "./settings.ts";

const MODES: Record<VoiceInputMode, { label: string; hint: string }> = {
  activity: {
    label: "Sprachaktivierung",
    hint: "Das Mikrofon sendet, sobald du sprichst.",
  },
  ptt: {
    label: "Push-to-Talk",
    hint: "Das Mikrofon sendet nur, solange du die Taste hältst.",
  },
};

export function PushToTalkSettings() {
  const voiceInput = useMediaSettings((s) => s.voiceInput);
  const pttKey = useMediaSettings((s) => s.pttKey);
  const pttKeyLabel = useMediaSettings((s) => s.pttKeyLabel);
  const releaseDelay = useMediaSettings((s) => s.pttReleaseDelay);
  const patch = useMediaSettings((s) => s.patch);
  const global = usePushToTalk((s) => s.global);
  const [recording, setRecording] = useState(false);
  const desktop = isDesktopApp();

  useEffect(() => {
    if (!recording) return;
    const take = (code: string, label: string) => {
      patch({ pttKey: code, pttKeyLabel: label });
      setRecording(false);
    };
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      if (event.code === "Escape") {
        setRecording(false);
        return;
      }
      const code = pushToTalkCode(event);
      if (!code) return;
      const label =
        event.key.length === 1 && event.key !== " "
          ? event.key.toUpperCase()
          : pushToTalkKeyLabel(code);
      take(code, label);
    };
    const onMouse = (event: MouseEvent) => {
      const code = pushToTalkMouseCode(event.button);
      if (!code) return;
      event.preventDefault();
      take(code, pushToTalkKeyLabel(code));
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("mousedown", onMouse, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("mousedown", onMouse, true);
    };
  }, [recording, patch]);

  return (
    <fieldset className="flex flex-col gap-3">
      <legend className="text-sm font-semibold text-lr-text">
        Sprachübertragung
      </legend>
      {(Object.keys(MODES) as VoiceInputMode[]).map((mode) => (
        <label key={mode} className="flex items-start gap-2 text-sm">
          <input
            type="radio"
            className="ui-check"
            name="voice-input"
            checked={voiceInput === mode}
            onChange={() => patch({ voiceInput: mode })}
          />
          <span>
            {MODES[mode].label}
            <small className="block text-lr-muted">{MODES[mode].hint}</small>
          </span>
        </label>
      ))}
      {voiceInput === "ptt" && (
        <>
          <div className="flex items-center gap-2 text-sm">
            <span className="font-medium text-lr-text">Taste</span>
            <button
              type="button"
              id="ptt-key"
              aria-live="polite"
              className="rounded border px-3 py-1.5 text-sm"
              onClick={() => setRecording((on) => !on)}
            >
              {recording
                ? "Taste oder Maustaste drücken … (Esc bricht ab)"
                : pttKey
                  ? pttKeyLabel || pushToTalkKeyLabel(pttKey)
                  : "Taste festlegen"}
            </button>
          </div>
          {!pttKey && (
            <p role="status" className="text-xs text-lr-muted">
              Ohne Taste bleibt dein Mikrofon im Gespräch stumm.
            </p>
          )}
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between gap-2 text-sm">
              <label
                htmlFor="ptt-release-delay"
                className="font-medium text-lr-text"
              >
                Nachlaufzeit
              </label>
              <span className="tabular-nums text-lr-muted">
                {releaseDelay} ms
              </span>
            </div>
            <input
              id="ptt-release-delay"
              type="range"
              min={0}
              max={PTT_RELEASE_DELAY_MAX}
              step={50}
              value={releaseDelay}
              onChange={(event) =>
                patch({ pttReleaseDelay: Number(event.target.value) })
              }
              className="w-full"
            />
            <p className="text-xs text-lr-muted">
              So lange sendet das Mikrofon nach dem Loslassen weiter, damit das
              letzte Wort nicht abreißt.
            </p>
          </div>
          {pttKey && (
            <p role="status" className="text-xs text-lr-muted">
              {desktop
                ? globalKeyNote(global, onWindows())
                : "Im Browser wirkt die Taste nur, solange Gelabber im Vordergrund ist. Die Desktop-App hört sie auch im Hintergrund."}
            </p>
          )}
        </>
      )}
    </fieldset>
  );
}
