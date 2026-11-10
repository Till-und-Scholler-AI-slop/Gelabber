// Voice activity or push-to-talk, the key and its release delay (pushToTalk.ts).

import { useEffect, useState } from "react";

import { Button } from "../components/ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import {
  PTT_RELEASE_DELAY_MAX,
  pushToTalkCode,
  pushToTalkKeyLabel,
  pushToTalkMouseCode,
  type VoiceInputMode,
} from "./pushToTalk.ts";
import { useMediaSettings } from "./settings.ts";
import { SettingRow, Slider } from "./settingsLayout.tsx";

const MODES = {
  activity: {
    label: "ptt.activity",
    hint: "ptt.activityHint",
  },
  ptt: {
    label: "ptt.ptt",
    hint: "ptt.pttHint",
  },
} as const satisfies Record<
  VoiceInputMode,
  { label: keyof typeof voiceSettings.de; hint: keyof typeof voiceSettings.de }
>;

export function PushToTalkSettings() {
  const t = useT(voiceSettings);
  const voiceInput = useMediaSettings((s) => s.voiceInput);
  const pttKey = useMediaSettings((s) => s.pttKey);
  const pttKeyLabel = useMediaSettings((s) => s.pttKeyLabel);
  const releaseDelay = useMediaSettings((s) => s.pttReleaseDelay);
  const patch = useMediaSettings((s) => s.patch);
  const [recording, setRecording] = useState(false);

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
      <SettingRow label={t("ptt.send")}>
        <ChipGroup<VoiceInputMode>
          label={t("ptt.modeLabel")}
          options={(Object.keys(MODES) as VoiceInputMode[]).map((mode) => ({
            value: mode,
            label: t(MODES[mode].label),
          }))}
          value={voiceInput}
          onChange={(mode) => patch({ voiceInput: mode })}
        />
        <p className="media-hint">{t(MODES[voiceInput].hint)}</p>
      </SettingRow>
      {voiceInput === "ptt" && (
        <SettingRow label={t("ptt.key")} htmlFor="ptt-key">
          <Button
            id="ptt-key"
            size="sm"
            className="self-start"
            aria-live="polite"
            onClick={() => setRecording((on) => !on)}
          >
            {recording
              ? t("ptt.recording")
              : pttKey
                ? pttKeyLabel || pushToTalkKeyLabel(pttKey)
                : t("ptt.setKey")}
          </Button>
          <p role="status" className="media-hint">
            {!pttKey ? t("ptt.noKeyHint") : t("ptt.keyHint")}
          </p>
        </SettingRow>
      )}
      {voiceInput === "ptt" && (
        <SettingRow label={t("ptt.releaseDelay")} htmlFor="ptt-release-delay">
          <Slider
            id="ptt-release-delay"
            min={0}
            max={PTT_RELEASE_DELAY_MAX}
            step={50}
            value={releaseDelay}
            suffix={`${releaseDelay} ms`}
            onChange={(value) => patch({ pttReleaseDelay: value })}
            hint={t("ptt.releaseDelayHint")}
          />
        </SettingRow>
      )}
    </>
  );
}
