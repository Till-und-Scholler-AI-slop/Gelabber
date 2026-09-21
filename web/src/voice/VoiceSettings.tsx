// Shared Voice/Video + notification form. Used on /settings and in-call.

import { useState } from "react";

import {
  AUDIO_QUALITY,
  type AudioQuality,
  type DeviceList,
  listMediaDevices,
  useMediaSettings,
} from "./settings.ts";

const emptyDevices: DeviceList = {
  audioinput: [],
  audiooutput: [],
  videoinput: [],
};

export function MediaSettingsForm() {
  const settings = useMediaSettings();
  const [devices, setDevices] = useState<DeviceList>(emptyDevices);

  const refresh = async () => {
    if (typeof navigator === "undefined" || !navigator.mediaDevices) {
      setDevices(emptyDevices);
      return;
    }
    try {
      const audio = await navigator.mediaDevices.getUserMedia({ audio: true });
      audio.getTracks().forEach((track) => track.stop());
    } catch {
      // mic labels stay anonymous until permission
    }
    try {
      const video = await navigator.mediaDevices.getUserMedia({ video: true });
      video.getTracks().forEach((track) => track.stop());
    } catch {
      // camera optional
    }
    setDevices(await listMediaDevices());
  };

  return (
    <div className="flex flex-col gap-6 text-left">
      <p className="text-sm text-neutral-600 dark:text-neutral-400">
        Defaults: Echo-Unterdrückung, Rauschunterdrückung und Auto-Gain an.
        Qualität <span className="font-medium">Normal</span> (64 kbit/s Opus).
        Geräteliste kommt vom Browser — kein fremdes Media-SDK.
      </p>

      <fieldset className="flex flex-col gap-3">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Geräte
        </legend>
        <Select
          id="audio-input"
          label="Mikrofon"
          value={settings.audioInputId}
          onChange={(value) => settings.patch({ audioInputId: value })}
          options={withCurrent(devices.audioinput, settings.audioInputId)}
        />
        <Select
          id="audio-output"
          label="Lautsprecher"
          value={settings.audioOutputId}
          onChange={(value) => settings.patch({ audioOutputId: value })}
          options={withCurrent(devices.audiooutput, settings.audioOutputId)}
        />
        <Select
          id="video-input"
          label="Kamera"
          value={settings.videoInputId}
          onChange={(value) => settings.patch({ videoInputId: value })}
          options={withCurrent(devices.videoinput, settings.videoInputId)}
        />
        <button
          type="button"
          onClick={() => void refresh()}
          className="self-start text-sm font-medium text-neutral-700 dark:text-neutral-300 underline-offset-2 hover:underline"
        >
          Geräte laden
        </button>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Verarbeitung
        </legend>
        <Toggle
          id="aec"
          label="Echo-Unterdrückung (AEC)"
          checked={settings.echoCancellation}
          onChange={(echoCancellation) => settings.patch({ echoCancellation })}
        />
        <Toggle
          id="ns"
          label="Rauschunterdrückung"
          checked={settings.noiseSuppression}
          onChange={(noiseSuppression) => settings.patch({ noiseSuppression })}
        />
        <Toggle
          id="agc"
          label="Auto-Gain"
          checked={settings.autoGainControl}
          onChange={(autoGainControl) => settings.patch({ autoGainControl })}
        />
      </fieldset>

      <fieldset className="flex flex-col gap-3">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Lautstärke
        </legend>
        <Slider
          id="output-volume"
          label="Wiedergabe"
          min={0}
          max={100}
          value={Math.round(settings.outputVolume * 100)}
          suffix={`${Math.round(settings.outputVolume * 100)} %`}
          onChange={(value) => settings.patch({ outputVolume: value / 100 })}
        />
        <Slider
          id="input-gain"
          label="Mic-Gain"
          min={0}
          max={200}
          value={Math.round(settings.inputGain * 100)}
          suffix={`${Math.round(settings.inputGain * 100)} %`}
          onChange={(value) => settings.patch({ inputGain: value / 100 })}
          hint="100 % sendet das Mikrofon unverändert (Default). Andere Werte laufen über Web Audio."
        />
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Audio-Qualität (Opus)
        </legend>
        {(Object.keys(AUDIO_QUALITY) as AudioQuality[]).map((key) => {
          const profile = AUDIO_QUALITY[key];
          return (
            <label key={key} className="flex items-center gap-2 text-sm">
              <input
                type="radio"
                name="audio-quality"
                checked={settings.quality === key}
                onChange={() => settings.patch({ quality: key })}
              />
              <span>
                {profile.label}
                <span className="text-neutral-500 dark:text-neutral-400">
                  {" "}
                  — {profile.hint}
                </span>
              </span>
            </label>
          );
        })}
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
          Nachrichten
        </legend>
        <Toggle
          id="message-toasts"
          label="Popup bei neuen Nachrichten in anderen Chats"
          checked={settings.messageToasts}
          onChange={(messageToasts) => settings.patch({ messageToasts })}
        />
        <Toggle
          id="desktop-notify"
          label="Browser-Benachrichtigung, wenn der Tab im Hintergrund ist"
          checked={settings.desktopNotify}
          onChange={(desktopNotify) => {
            settings.patch({ desktopNotify });
            if (desktopNotify && typeof Notification !== "undefined") {
              void Notification.requestPermission();
            }
          }}
        />
      </fieldset>
    </div>
  );
}

function withCurrent(
  options: { id: string; label: string }[],
  value: string,
): { id: string; label: string }[] {
  if (!value || options.some((option) => option.id === value)) return options;
  return [{ id: value, label: "Gewähltes Gerät" }, ...options];
}

function Select({
  id,
  label,
  value,
  onChange,
  options,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { id: string; label: string }[];
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label
        htmlFor={id}
        className="text-sm font-medium text-neutral-800 dark:text-neutral-200"
      >
        {label}
      </label>
      <select
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className="rounded-lg border border-neutral-300 dark:border-neutral-600 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-neutral-900 dark:text-neutral-100 outline-none focus:border-neutral-500 dark:focus:border-neutral-400 focus:ring-2 focus:ring-neutral-200 dark:focus:ring-neutral-700"
      >
        <option value="">Browser-Default</option>
        {options.map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </select>
    </div>
  );
}

function Toggle({
  id,
  label,
  checked,
  onChange,
}: {
  id: string;
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label htmlFor={id} className="flex items-center gap-2 text-sm">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}

function Slider({
  id,
  label,
  min,
  max,
  value,
  suffix,
  onChange,
  hint,
}: {
  id: string;
  label: string;
  min: number;
  max: number;
  value: number;
  suffix: string;
  onChange: (value: number) => void;
  hint?: string;
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2 text-sm">
        <label
          htmlFor={id}
          className="font-medium text-neutral-800 dark:text-neutral-200"
        >
          {label}
        </label>
        <span className="tabular-nums text-neutral-500 dark:text-neutral-400">
          {suffix}
        </span>
      </div>
      <input
        id={id}
        type="range"
        min={min}
        max={max}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
        className="w-full"
      />
      {hint ? (
        <p className="text-xs text-neutral-500 dark:text-neutral-400">{hint}</p>
      ) : null}
    </div>
  );
}
