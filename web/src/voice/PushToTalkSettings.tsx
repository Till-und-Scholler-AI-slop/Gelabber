// Voice activity or push-to-talk, the key and its release delay (pushToTalk.ts).

import { useEffect, useState } from "react";

import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
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
import { SettingRow, Slider } from "./settingsLayout.tsx";

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
    <>
      <SettingRow label="Senden">
        <ChipGroup<VoiceInputMode>
          label="Sprachübertragung"
          options={(Object.keys(MODES) as VoiceInputMode[]).map((mode) => ({
            value: mode,
            label: MODES[mode].label,
          }))}
          value={voiceInput}
          onChange={(mode) => patch({ voiceInput: mode })}
        />
        <p className="media-hint">{MODES[voiceInput].hint}</p>
      </SettingRow>
      {voiceInput === "ptt" && (
        <SettingRow label="Taste" htmlFor="ptt-key">
          <Button
            id="ptt-key"
            size="sm"
            className="self-start"
            aria-live="polite"
            onClick={() => setRecording((on) => !on)}
          >
            {recording
              ? "Taste oder Maustaste drücken … (Esc bricht ab)"
              : pttKey
                ? pttKeyLabel || pushToTalkKeyLabel(pttKey)
                : "Taste festlegen"}
          </Button>
          <p role="status" className="media-hint">
            {!pttKey
              ? "Ohne Taste bleibt dein Mikrofon im Gespräch stumm."
              : desktop
                ? globalKeyNote(global, onWindows())
                : "Im Browser wirkt die Taste nur, solange Gelabber im Vordergrund ist. Die Desktop-App hört sie auch im Hintergrund."}
          </p>
        </SettingRow>
      )}
      {voiceInput === "ptt" && (
        <SettingRow label="Nachlaufzeit" htmlFor="ptt-release-delay">
          <Slider
            id="ptt-release-delay"
            min={0}
            max={PTT_RELEASE_DELAY_MAX}
            step={50}
            value={releaseDelay}
            suffix={`${releaseDelay} ms`}
            onChange={(value) => patch({ pttReleaseDelay: value })}
            hint="So lange sendet das Mikrofon nach dem Loslassen weiter, damit das letzte Wort nicht abreißt."
          />
        </SettingRow>
      )}
    </>
  );
}
