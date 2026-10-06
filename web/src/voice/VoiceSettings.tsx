import { useAudioProcessing } from "./audioProcessing.ts";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
// Shared Voice/Video + notification form. Used on /settings and in-call.

import { useState, type ReactNode } from "react";
import "./quality.css";
import { playCallSound } from "./callSounds.ts";

import {
  AUDIO_PROCESSING,
  audioBitrate,
  type AudioProcessingMode,
  type DeviceList,
  type StreamApply,
  type StreamKind,
  type StreamProfileId,
  STREAM_PROFILES,
  VIDEO_RESOLUTIONS,
  VIDEO_FRAME_RATES,
  explicitStreamProfile,
  type VideoResolution,
  type VideoFrameRate,
  formatVideoBitrate,
  listMediaDevices,
  useMediaSettings,
  videoSendBudget,
} from "./settings.ts";

const emptyDevices: DeviceList = {
  audioinput: [],
  audiooutput: [],
  videoinput: [],
};

type SettingsSection = "all" | "audio" | "video" | "notifications";

export function MediaSettingsForm({
  section = "all",
}: {
  section?: SettingsSection;
}) {
  const showAudio = section === "all" || section === "audio";
  const showVideo = section === "all" || section === "video";
  const showNotifications = section === "all" || section === "notifications";
  const settings = useMediaSettings();
  const processing = useAudioProcessing();
  const audioLimit = audioBitrate(settings);
  const videoLimit = videoSendBudget(settings);
  const [microphoneTest, setMicrophoneTest] = useState(false);
  const [devices, setDevices] = useState<DeviceList>(emptyDevices);

  const refresh = async () => {
    if (typeof navigator === "undefined" || !navigator.mediaDevices) {
      setDevices(emptyDevices);
      return;
    }
    if (showAudio) {
      try {
        const audio = await navigator.mediaDevices.getUserMedia({
          audio: true,
        });
        audio.getTracks().forEach((track) => track.stop());
      } catch {
        // mic labels stay anonymous until permission
      }
    }
    if (showVideo) {
      try {
        const video = await navigator.mediaDevices.getUserMedia({
          video: true,
        });
        video.getTracks().forEach((track) => track.stop());
      } catch {
        // camera optional
      }
    }
    setDevices(await listMediaDevices());
  };

  return (
    <div className="media-settings-form flex flex-col gap-6 text-left">
      {section === "all" && (
        <p className="text-sm text-neutral-600 dark:text-neutral-400">
          Passe Mikrofon, Lautsprecher und Kamera an. Die Übertragung hat
          standardmäßig keine Bitratengrenze durch Gelabber.
        </p>
      )}

      {(showAudio || showVideo) && (
        <fieldset className="flex flex-col gap-3">
          <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
            Geräte
          </legend>
          {showAudio && (
            <Select
              id="audio-input"
              label="Mikrofon"
              value={settings.audioInputId}
              onChange={(value) => settings.patch({ audioInputId: value })}
              options={withCurrent(devices.audioinput, settings.audioInputId)}
            />
          )}
          {showAudio && (
            <Select
              id="audio-output"
              label="Lautsprecher"
              value={settings.audioOutputId}
              onChange={(value) => settings.patch({ audioOutputId: value })}
              options={withCurrent(devices.audiooutput, settings.audioOutputId)}
            />
          )}
          {(showAudio || showVideo) && (
            <fieldset className="flex flex-col gap-2">
              <legend className="text-sm font-semibold">Bandbreite</legend>
              <Toggle
                id="economy-mode"
                label="Sparmodus für wenig Upload"
                checked={settings.economyMode}
                onChange={(economyMode) =>
                  settings.patch({ economyMode, quality: "normal" })
                }
              />
              <p className="text-xs text-neutral-500">
                {settings.economyMode
                  ? `Ausdrückliche Obergrenzen: Sprache ${audioLimit! / 1000} kbit/s, Stream-Ton 128 kbit/s, Video gemeinsam ${formatVideoBitrate(videoLimit!)}.`
                  : videoLimit
                    ? `Audio ohne Bitratengrenze durch Gelabber; Video mit deinem gemeinsamen Limit von ${formatVideoBitrate(videoLimit)}.`
                    : "Keine Bitratengrenze durch Gelabber. Browser, Codec und Verbindung bestimmen die tatsächliche Datenrate."}
              </p>
            </fieldset>
          )}
          {showVideo && (
            <Select
              id="video-input"
              label="Kamera"
              value={settings.videoInputId}
              onChange={(value) => settings.patch({ videoInputId: value })}
              options={withCurrent(devices.videoinput, settings.videoInputId)}
            />
          )}
          <button
            type="button"
            onClick={() => void refresh()}
            className="self-start text-sm font-medium text-neutral-700 dark:text-neutral-300 underline-offset-2 hover:underline"
          >
            Geräte laden
          </button>
        </fieldset>
      )}
      {showAudio && (
        <>
          <fieldset className="flex flex-col gap-3">
            <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
              Lautstärke
            </legend>
            <Slider
              id="output-volume"
              label="Gespräche"
              min={0}
              max={100}
              value={Math.round(settings.outputVolume * 100)}
              suffix={`${Math.round(settings.outputVolume * 100)} %`}
              onChange={(value) =>
                settings.patch({ outputVolume: value / 100 })
              }
            />
            <Slider
              id="source-audio-volume"
              label="Stream-Ton"
              min={0}
              max={100}
              value={Math.round(settings.sourceAudioVolume * 100)}
              suffix={`${Math.round(settings.sourceAudioVolume * 100)} %`}
              onChange={(value) =>
                settings.patch({ sourceAudioVolume: value / 100 })
              }
              hint="Bildschirm- und Live-Ton haben eine eigene Lautstärke. Taub schaltet auch diesen Ton aus."
            />
            <Toggle
              id="source-audio-muted"
              label="Stream-Ton stummschalten"
              checked={settings.sourceAudioMuted}
              onChange={(sourceAudioMuted) =>
                settings.patch({ sourceAudioMuted })
              }
            />
            <Slider
              id="input-gain"
              label="Mic-Gain"
              min={0}
              max={200}
              value={Math.round(settings.inputGain * 100)}
              suffix={`${Math.round(settings.inputGain * 100)} %`}
              onChange={(value) => settings.patch({ inputGain: value / 100 })}
              hint="100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio."
            />
          </fieldset>
          <fieldset className="flex flex-col gap-3">
            <legend className="text-sm font-semibold">
              Mikrofonverarbeitung
            </legend>
            {(Object.keys(AUDIO_PROCESSING) as AudioProcessingMode[]).map(
              (mode) => (
                <label key={mode} className="flex items-start gap-2 text-sm">
                  <input
                    type="radio"
                    name="audio-processing"
                    checked={settings.processingMode === mode}
                    onChange={() => settings.patch({ processingMode: mode })}
                  />
                  <span>
                    {AUDIO_PROCESSING[mode].label}
                    <small className="block text-neutral-500">
                      {AUDIO_PROCESSING[mode].hint}
                    </small>
                  </span>
                </label>
              ),
            )}
            <p role="status" className="text-xs text-neutral-500">
              {processing.message}
            </p>
            <button
              type="button"
              className="self-start rounded border px-3 py-2 text-sm"
              onClick={() => setMicrophoneTest(true)}
            >
              Mikrofon testen und vergleichen
            </button>
          </fieldset>
          <AdvancedAudio expanded={section === "all"}>
            <fieldset className="flex flex-col gap-2">
              <legend className="text-sm font-semibold">
                Echo und Browserfilter
              </legend>
              <Toggle
                id="aec"
                label="Echo-Unterdrückung (AEC)"
                checked={settings.echoCancellation}
                onChange={(echoCancellation) =>
                  settings.patch({ echoCancellation })
                }
              />
              <p className="text-xs text-neutral-500">
                Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du
                sie ausschalten.
              </p>
              {settings.processingMode === "browser" && (
                <>
                  <Toggle
                    id="ns"
                    label="Rauschunterdrückung des Browsers"
                    checked={settings.noiseSuppression}
                    onChange={(noiseSuppression) =>
                      settings.patch({ noiseSuppression })
                    }
                  />
                  <Toggle
                    id="agc"
                    label="Auto-Gain des Browsers"
                    checked={settings.autoGainControl}
                    onChange={(autoGainControl) =>
                      settings.patch({ autoGainControl })
                    }
                  />
                </>
              )}
            </fieldset>
          </AdvancedAudio>
        </>
      )}
      {showVideo && (
        <fieldset className="flex flex-col gap-4">
          <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
            Stream-Qualität
          </legend>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            Wähle Auflösung und Bildrate für deine Übertragung. Höhere Werte
            brauchen mehr Rechenleistung und Upload. Die erreichbare Qualität
            hängt von Quelle, Gerät und Verbindung ab.
          </p>
          <StreamProfilePicker
            kind="camera"
            name="camera-stream-profile"
            legend="Kamera"
            value={settings.cameraProfile}
            apply={settings.cameraProfileApply}
            onChange={(cameraProfile) => settings.patch({ cameraProfile })}
          />
          <StreamProfilePicker
            kind="screen"
            name="screen-stream-profile"
            legend="Bildschirmfreigabe und Go Live"
            value={settings.screenProfile}
            apply={settings.screenProfileApply}
            onChange={(screenProfile) => settings.patch({ screenProfile })}
          />
          <section
            className="stream-source-audio"
            aria-label="Ton der Bildschirmfreigabe"
          >
            <Toggle
              id="share-source-audio"
              label="Ton teilen"
              checked={settings.shareSourceAudio}
              onChange={(shareSourceAudio) =>
                settings.patch({ shareSourceAudio })
              }
            />
            <p className="text-xs text-neutral-500 dark:text-neutral-400">
              Gilt für die nächste Bildschirmfreigabe und Go Live. Wähle den Ton
              im Browserdialog aus; je nach Browser und Quelle ist nur Video
              verfügbar. Der Stream-Ton wird als Stereo-Musik übertragen, ohne
              Mikrofonfilter.
            </p>
          </section>
          <section className="stream-upload" aria-label="Video-Upload">
            <div className="stream-quality-heading">
              <h3>Video-Upload</h3>
              <span>
                {videoLimit ? formatVideoBitrate(videoLimit) : "Ohne Limit"}
              </span>
            </div>
            <div className="stream-upload-mode">
              <label>
                <input
                  type="radio"
                  name="video-upload-mode"
                  checked={settings.videoUploadLimit === 0}
                  onChange={() => settings.patch({ videoUploadLimit: 0 })}
                />{" "}
                {settings.economyMode ? "Sparmodus (2,5 Mbit/s)" : "Ohne Limit"}
              </label>
              <label>
                <input
                  type="radio"
                  name="video-upload-mode"
                  checked={settings.videoUploadLimit > 0}
                  onChange={() =>
                    settings.patch({ videoUploadLimit: 10_000_000 })
                  }
                />{" "}
                Eigenes Limit
              </label>
            </div>
            {settings.videoUploadLimit > 0 && (
              <div className="flex flex-col gap-2">
                <label htmlFor="video-upload-limit">
                  Eigenes gemeinsames Video-Limit (Mbit/s)
                </label>
                <input
                  id="video-upload-limit"
                  type="number"
                  min="0.001"
                  max="4294.967"
                  step="any"
                  value={settings.videoUploadLimit / 1_000_000}
                  onChange={(event) =>
                    settings.patch({
                      videoUploadLimit: Number(event.target.value) * 1_000_000,
                    })
                  }
                  className="rounded border px-3 py-2"
                />
              </div>
            )}
            <p className="stream-quality-budget">
              Der Browser passt die tatsächliche Bitrate an die Verbindung an.
              Das Limit ist eine Obergrenze, keine feste Datenrate.
            </p>
          </section>
          <p className="stream-quality-budget">
            Änderungen gelten beim nächsten Start und werden, soweit
            unterstützt, auch auf laufende Streams angewendet.
          </p>
        </fieldset>
      )}
      {(showAudio || showNotifications) && (
        <fieldset className="flex flex-col gap-2">
          <legend className="text-sm font-semibold text-neutral-800 dark:text-neutral-200">
            Call-Sounds
          </legend>
          <Toggle
            id="call-sounds"
            label="Signaltöne im Call"
            checked={settings.callSounds}
            onChange={(callSounds) => settings.patch({ callSounds })}
          />
          <p className="text-sm text-neutral-600 dark:text-neutral-400">
            Kurze Töne beim Beitreten und Verlassen sowie für dein Mikrofon und
            Taubstellen. Wenn du taubgestellt bist, bleiben Teilnehmer-Töne
            stumm.
          </p>
          <Slider
            id="call-sound-volume"
            label="Lautstärke der Signaltöne"
            min={0}
            max={1}
            step={0.05}
            value={settings.callSoundVolume}
            suffix={`${Math.round(settings.callSoundVolume * 100)} %`}
            onChange={(callSoundVolume) => settings.patch({ callSoundVolume })}
            hint="Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke."
          />
          <button
            type="button"
            className="self-start rounded border border-neutral-300 px-3 py-2 text-sm font-medium text-neutral-800 hover:bg-neutral-100 disabled:opacity-50 dark:border-neutral-600 dark:text-neutral-200 dark:hover:bg-neutral-800"
            disabled={
              !settings.callSounds ||
              settings.callSoundVolume === 0 ||
              settings.outputVolume === 0
            }
            onClick={() => playCallSound("join")}
          >
            Testton abspielen
          </button>
        </fieldset>
      )}
      {microphoneTest && (
        <MicrophoneTest onClose={() => setMicrophoneTest(false)} />
      )}
      {showNotifications && (
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
      )}
    </div>
  );
}

function AdvancedAudio({
  expanded,
  children,
}: {
  expanded: boolean;
  children: ReactNode;
}) {
  if (expanded) return <>{children}</>;
  return (
    <details className="settings-advanced">
      <summary>Erweiterte Audio-Einstellungen</summary>
      <div className="flex flex-col gap-6">{children}</div>
    </details>
  );
}

function StreamProfilePicker({
  kind,
  name,
  legend,
  value,
  apply,
  onChange,
}: {
  kind: StreamKind;
  name: string;
  legend: string;
  value: StreamProfileId;
  apply: StreamApply;
  onChange: (value: StreamProfileId) => void;
}) {
  const profile = STREAM_PROFILES[value];
  const automatic = value === "balanced";
  const height = (
    automatic && kind === "screen" ? 1080 : profile.height
  ) as VideoResolution;
  const fps = profile.fps as VideoFrameRate;
  return (
    <section className="stream-quality-picker" aria-label={legend}>
      <header className="stream-quality-heading">
        <h3>{legend}</h3>
        <span>
          {automatic
            ? "Automatisch"
            : `${height === 2160 ? "4K" : `${height}p`} · ${fps} FPS`}
        </span>
      </header>
      <div className="stream-quality-fields">
        <Select
          defaultLabel={null}
          id={`${name}-resolution`}
          label="Auflösung"
          value={automatic ? "auto" : String(height)}
          onChange={(next) =>
            onChange(
              next === "auto"
                ? "balanced"
                : explicitStreamProfile(Number(next) as VideoResolution, fps),
            )
          }
          options={[
            { id: "auto", label: "Automatisch (bis 1080p)" },
            ...VIDEO_RESOLUTIONS.map((resolution) => ({
              id: String(resolution),
              label: resolution === 2160 ? "4K · 2160p" : `${resolution}p`,
            })),
          ]}
        />
        <Select
          defaultLabel={null}
          id={`${name}-fps`}
          label="Bildrate"
          value={automatic ? "auto" : String(fps)}
          onChange={(next) =>
            onChange(
              next === "auto"
                ? "balanced"
                : explicitStreamProfile(height, Number(next) as VideoFrameRate),
            )
          }
          options={[
            ...(automatic
              ? [{ id: "auto", label: "Automatisch (bis 30 FPS)" }]
              : []),
            ...VIDEO_FRAME_RATES.map((rate) => ({
              id: String(rate),
              label: `${rate} FPS`,
            })),
          ]}
        />
      </div>
      <ApplyNote apply={apply} />
    </section>
  );
}

function ApplyNote({ apply }: { apply: StreamApply }) {
  if (apply === "live") {
    return (
      <p className="text-xs text-neutral-600 dark:text-neutral-400">
        Auf den laufenden Stream angewendet.
      </p>
    );
  }
  if (apply === "next") {
    return (
      <p className="text-xs text-neutral-600 dark:text-neutral-400">
        Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten
        Stream.
      </p>
    );
  }
  return null;
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
  defaultLabel = "Browser-Default",
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { id: string; label: string }[];
  defaultLabel?: string | null;
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
        {defaultLabel !== null && <option value="">{defaultLabel}</option>}
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
  step,
  onChange,
  hint,
}: {
  id: string;
  label: string;
  min: number;
  max: number;
  value: number;
  suffix: string;
  step?: number;
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
        step={step}
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
