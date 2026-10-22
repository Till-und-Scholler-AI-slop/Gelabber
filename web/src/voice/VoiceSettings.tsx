import { useAudioProcessing } from "./audioProcessing.ts";
import { canChooseSpeaker, useCapabilities } from "./capabilities.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { listNativeAudioApps } from "./native/capture.ts";
import { hasNativeFeature } from "./native/features.ts";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { PushToTalkSettings } from "./PushToTalkSettings.tsx";
import {
  deviceChoices,
  needsDeviceLabels,
  useMediaDevices,
} from "./devices.ts";
// Shared Voice/Video + notification form. Used on /settings and in-call.

import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import { Button } from "../components/ui/Button.tsx";
import { Input } from "../components/ui/Input.tsx";
import { Select as UiSelect } from "../components/ui/Select.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import "./quality.css";
import { playCallSound } from "./callSounds.ts";
import { CloseToTray } from "../components/CloseToTray.tsx";

import {
  AUDIO_PROCESSING,
  AUDIO_PROCESSING_DESKTOP,
  audioBitrate,
  type AudioProcessingMode,
  type DeviceOption,
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
  MAX_PLAYBACK_VOLUME,
  sharesSourceAudio,
  SOURCE_AUDIO_CARRIES_CALL,
  sourceAudioCarriesCall,
  sourceAudioChoice,
  useMediaSettings,
  videoSendBudget,
} from "./settings.ts";

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
  const { devices, loaded, reveal } = useMediaDevices();
  const [revealFailed, setRevealFailed] = useState(false);
  // The desktop app's native core captures, filters and sends; the texts
  // must not credit a browser with it.
  const desktop = isDesktopApp();
  const chooseSpeaker = canChooseSpeaker();
  const capable = useCapabilities();
  const processingModes = desktop ? AUDIO_PROCESSING_DESKTOP : AUDIO_PROCESSING;

  return (
    <div className="media-settings-form flex flex-col gap-6 text-left">
      {section === "all" && (
        <p className="text-sm text-lr-muted">
          Passe Mikrofon, Lautsprecher und Kamera an. Die Übertragung hat
          standardmäßig keine Bitratengrenze durch Gelabber.
        </p>
      )}

      {(showAudio || showVideo) && (
        <fieldset className="flex flex-col gap-3">
          <legend className="text-sm font-semibold text-lr-text">Geräte</legend>
          {showAudio && (
            <DeviceSelect
              id="audio-input"
              label="Mikrofon"
              devices={devices.audioinput}
              value={settings.audioInputId}
              onChange={(value) => settings.patch({ audioInputId: value })}
            />
          )}
          {showAudio && chooseSpeaker && (
            <DeviceSelect
              id="audio-output"
              label="Lautsprecher"
              devices={devices.audiooutput}
              value={settings.audioOutputId}
              onChange={(value) => settings.patch({ audioOutputId: value })}
            />
          )}
          {showAudio && !chooseSpeaker && (
            <p className="text-xs text-lr-muted">
              Dieser Browser kann den Lautsprecher nicht wählen. Der Ton folgt
              der Ausgabe deines Geräts.
            </p>
          )}
          {showAudio &&
            loaded &&
            !desktop &&
            needsDeviceLabels(devices.audioinput) && (
              <div className="media-settings-reveal">
                <p className="text-xs text-lr-muted">
                  {revealFailed
                    ? "Der Browser gibt die Gerätenamen ohne Mikrofonzugriff nicht heraus."
                    : "Die Gerätenamen zeigt der Browser erst, wenn du den Mikrofonzugriff erlaubst."}
                </p>
                <Button
                  size="sm"
                  onClick={() =>
                    void reveal("audio").then((ok) => setRevealFailed(!ok))
                  }
                >
                  Gerätenamen anzeigen
                </Button>
              </div>
            )}
          {showVideo && (
            <DeviceSelect
              id="video-input"
              label="Kamera"
              devices={devices.videoinput}
              value={settings.videoInputId}
              onChange={(value) => settings.patch({ videoInputId: value })}
            />
          )}
          {showVideo &&
            loaded &&
            !desktop &&
            needsDeviceLabels(devices.videoinput) && (
              <Button
                size="sm"
                variant="ghost"
                className="self-start"
                onClick={() => void reveal("video")}
              >
                Kameranamen anzeigen
              </Button>
            )}
        </fieldset>
      )}
      {(showAudio || showVideo) && (
        <fieldset className="flex flex-col gap-2">
          <legend className="text-sm font-semibold text-lr-text">
            Bandbreite
          </legend>
          <Toggle
            id="economy-mode"
            label="Sparmodus für wenig Upload"
            checked={settings.economyMode}
            onChange={(economyMode) =>
              settings.patch({ economyMode, quality: "normal" })
            }
          />
          <p className="text-xs text-lr-muted">
            {settings.economyMode
              ? `Ausdrückliche Obergrenzen: Sprache ${audioLimit! / 1000} kbit/s, Stream-Ton 128 kbit/s, Video gemeinsam ${formatVideoBitrate(videoLimit!)}.`
              : videoLimit
                ? `Audio ohne Bitratengrenze durch Gelabber; Video mit deinem gemeinsamen Limit von ${formatVideoBitrate(videoLimit)}.`
                : desktop
                  ? "Keine Bitratengrenze durch Gelabber. Codec und Verbindung bestimmen die tatsächliche Datenrate."
                  : "Keine Bitratengrenze durch Gelabber. Browser, Codec und Verbindung bestimmen die tatsächliche Datenrate."}
          </p>
        </fieldset>
      )}
      {showAudio && (
        <>
          <fieldset className="flex flex-col gap-3">
            <legend className="text-sm font-semibold text-lr-text">
              Lautstärke
            </legend>
            <Slider
              id="output-volume"
              label="Gespräche"
              min={0}
              max={MAX_PLAYBACK_VOLUME * 100}
              value={Math.round(settings.outputVolume * 100)}
              suffix={`${Math.round(settings.outputVolume * 100)} %`}
              onChange={(value) =>
                settings.patch({ outputVolume: value / 100 })
              }
              hint={
                desktop
                  ? "Über 100 % wird der Ton digital verstärkt."
                  : "Über 100 % wird der Ton digital verstärkt. Nutze dafür Kopfhörer: Über Lautsprecher können die anderen sonst ihr eigenes Echo hören."
              }
            />
            <Slider
              id="source-audio-volume"
              label="Stream-Ton"
              min={0}
              max={MAX_PLAYBACK_VOLUME * 100}
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
              scale={["0 %", "100 %", "200 %"]}
              onChange={(value) => settings.patch({ inputGain: value / 100 })}
              hint={
                desktop
                  ? "100 % fügt keine Verstärkung hinzu."
                  : "100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio."
              }
            />
          </fieldset>
          <PushToTalkSettings />
          <fieldset className="flex flex-col gap-3">
            <legend className="text-sm font-semibold">
              Mikrofonverarbeitung
            </legend>
            {(Object.keys(processingModes) as AudioProcessingMode[]).map(
              (mode) => (
                <label key={mode} className="flex items-start gap-2 text-sm">
                  <input
                    type="radio"
                    className="ui-check"
                    name="audio-processing"
                    checked={settings.processingMode === mode}
                    onChange={() => settings.patch({ processingMode: mode })}
                  />
                  <span>
                    {processingModes[mode].label}
                    <small className="block text-lr-muted">
                      {processingModes[mode].hint}
                    </small>
                  </span>
                </label>
              ),
            )}
            <p role="status" className="text-xs text-lr-muted">
              {processing.message}
            </p>
            {/* The desktop app's test shows the native core's meters; the
                A/B recording needs Web Audio. */}
            <Button
              size="sm"
              className="self-start"
              onClick={() => setMicrophoneTest(true)}
            >
              {desktop ? "Mikrofon testen" : "Mikrofon testen und vergleichen"}
            </Button>
          </fieldset>
          <AdvancedAudio expanded={section === "all"}>
            <fieldset className="flex flex-col gap-2">
              <legend className="text-sm font-semibold">
                {desktop ? "Echo und WebRTC-Filter" : "Echo und Browserfilter"}
              </legend>
              <Toggle
                id="aec"
                label="Echo-Unterdrückung (AEC)"
                checked={settings.echoCancellation}
                onChange={(echoCancellation) =>
                  settings.patch({ echoCancellation })
                }
              />
              <p className="text-xs text-lr-muted">
                Bei Lautsprechern empfohlen. Für Musik mit Kopfhörern kannst du
                sie ausschalten.
              </p>
              {settings.processingMode === "browser" && (
                <Toggle
                  id="ns"
                  label={
                    desktop
                      ? "Rauschunterdrückung von WebRTC"
                      : "Rauschunterdrückung des Browsers"
                  }
                  checked={settings.noiseSuppression}
                  onChange={(noiseSuppression) =>
                    settings.patch({ noiseSuppression })
                  }
                />
              )}
              {settings.processingMode !== "original" && (
                <>
                  <Toggle
                    id="agc"
                    label={
                      desktop
                        ? "Auto-Gain von WebRTC"
                        : "Auto-Gain des Browsers"
                    }
                    checked={settings.autoGainControl}
                    onChange={(autoGainControl) =>
                      settings.patch({ autoGainControl })
                    }
                  />
                  <p className="text-xs text-lr-muted">
                    Gleicht leise und laute Mikrofone auf einen hörbaren Pegel
                    an.
                  </p>
                </>
              )}
            </fieldset>
          </AdvancedAudio>
        </>
      )}
      {showVideo && (
        <fieldset className="flex flex-col gap-4">
          <legend className="text-sm font-semibold text-lr-text">
            Stream-Qualität
          </legend>
          <p className="text-xs text-lr-muted">
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
          {/* Only where a share can carry sound; the same switch sits next
              to the share button in the call controls. */}
          {capable.appAudio && (
            <section
              className="stream-source-audio"
              aria-label="Ton der Bildschirmfreigabe"
            >
              <Toggle
                id="share-source-audio"
                label="Ton teilen"
                checked={sharesSourceAudio(settings)}
                onChange={(on) =>
                  settings.patch({ sourceAudioShare: sourceAudioChoice(on) })
                }
              />
              {desktop && (
                <SourceAudioApp
                  value={settings.sourceAudioApp}
                  onChange={(sourceAudioApp) =>
                    settings.patch({ sourceAudioApp })
                  }
                />
              )}
              {sourceAudioCarriesCall(settings) && (
                <p
                  role="status"
                  className="stream-source-audio-warning text-xs"
                >
                  {SOURCE_AUDIO_CARRIES_CALL}
                </p>
              )}
              <p className="text-xs text-lr-muted">
                {!desktop
                  ? "Gilt für die nächste Bildschirmfreigabe und Go Live; Ausschalten wirkt sofort. Wähle den Ton im Browserdialog aus; je nach Browser und Quelle ist nur Video verfügbar."
                  : hasNativeFeature("app-audio-excludes-self")
                    ? "Gilt für Bildschirmfreigabe und Go Live, auch während sie laufen. Geteilt wird der Ton anderer Anwendungen, nie dein Mikrofon oder der Ton des Anrufs."
                    : "Gilt für Bildschirmfreigabe und Go Live, auch während sie laufen. Geteilt wird der Ton der gewählten Anwendung, nie dein Mikrofon."}{" "}
                Der Stream-Ton wird als Stereo-Musik übertragen, ohne
                Mikrofonfilter.
              </p>
            </section>
          )}
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
                  className="ui-check"
                  name="video-upload-mode"
                  checked={settings.videoUploadLimit === 0}
                  onChange={() => settings.patch({ videoUploadLimit: 0 })}
                />{" "}
                {settings.economyMode ? "Sparmodus (2,5 Mbit/s)" : "Ohne Limit"}
              </label>
              <label>
                <input
                  type="radio"
                  className="ui-check"
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
                <Input
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
                />
              </div>
            )}
            <p className="stream-quality-budget">
              {desktop ? "Die Desktop-App" : "Der Browser"} passt die
              tatsächliche Bitrate an die Verbindung an. Das Limit ist eine
              Obergrenze, keine feste Datenrate.
            </p>
          </section>
          <p className="stream-quality-budget">
            Änderungen gelten beim nächsten Start und werden, soweit
            unterstützt, auch auf laufende Streams angewendet.
          </p>
        </fieldset>
      )}
      {showAudio && (
        <fieldset className="flex flex-col gap-2">
          <legend className="text-sm font-semibold text-lr-text">
            Call-Sounds
          </legend>
          <Toggle
            id="call-sounds"
            label="Signaltöne im Call"
            checked={settings.callSounds}
            onChange={(callSounds) => settings.patch({ callSounds })}
          />
          <p className="text-sm text-lr-muted">
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
            hint={
              desktop
                ? "Nutzt die Wiedergabelautstärke. Signaltöne spielen auf dem Standard-Ausgabegerät des Systems."
                : chooseSpeaker
                  ? "Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke."
                  : "Nutzt die Wiedergabelautstärke."
            }
          />
          <Button
            size="sm"
            className="self-start"
            disabled={
              !settings.callSounds ||
              settings.callSoundVolume === 0 ||
              settings.outputVolume === 0
            }
            onClick={() => playCallSound("join")}
          >
            Testton abspielen
          </Button>
        </fieldset>
      )}
      {microphoneTest && (
        <MicrophoneTest onClose={() => setMicrophoneTest(false)} />
      )}
      {showNotifications && (
        <fieldset className="flex flex-col gap-2">
          <legend className="text-sm font-semibold text-lr-text">
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
            label={
              desktop
                ? "Benachrichtigung, wenn das Gelabber-Fenster nicht im Vordergrund ist"
                : "Browser-Benachrichtigung, wenn der Tab nicht im Vordergrund ist"
            }
            checked={settings.desktopNotify}
            onChange={(desktopNotify) => {
              settings.patch({ desktopNotify });
              if (desktopNotify && typeof Notification !== "undefined") {
                void Notification.requestPermission();
              }
            }}
          />
          <Toggle
            id="message-sound"
            label="Ton bei Benachrichtigungen am Computer"
            checked={settings.messageSound}
            disabled={!settings.desktopNotify}
            onChange={(messageSound) => settings.patch({ messageSound })}
          />
          <p className="text-xs text-lr-muted">
            Höchstens ein Ton für mehrere Nachrichten kurz hintereinander,
            keiner während du taub geschaltet bist, so laut wie die Töne für
            Gespräche. Auf dem Handy klingt die Benachrichtigung so, wie das
            Gerät eingestellt ist.
          </p>
          {desktop && <CloseToTray />}
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
      <p className="text-xs text-lr-muted">
        Auf den laufenden Stream angewendet.
      </p>
    );
  }
  if (apply === "next") {
    return (
      <p className="text-xs text-lr-muted">
        Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten
        Stream.
      </p>
    );
  }
  return null;
}

/** Desktop app: whose sound a share carries. Lists the applications playing
 * right now; a saved choice that is quiet at the moment stays selectable. */
function SourceAudioApp({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const [apps, setApps] = useState<{ id: string; label: string }[]>([]);
  const load = () =>
    void listNativeAudioApps()
      .then((list) =>
        setApps(list.map((app) => ({ id: app.id, label: app.name || app.id }))),
      )
      .catch(() => setApps([]));
  useEffect(load, []);
  // An app before v0.6 saved the application's name as its id. The core
  // still takes it; the list shows the application it means.
  const chosen = apps.some((app) => app.id === value)
    ? value
    : (apps.find((app) => app.label === value)?.id ?? value);
  return (
    <div onFocus={load}>
      <Select
        id="source-audio-app"
        label="Ton von"
        value={chosen}
        onChange={onChange}
        options={
          chosen && !apps.some((app) => app.id === chosen)
            ? [{ id: chosen, label: `${chosen} (gerade still)` }, ...apps]
            : apps
        }
        // An app that captures its own playout leaves nothing out.
        defaultLabel={
          hasNativeFeature("app-audio-excludes-self")
            ? "Alle Anwendungen außer Gelabber"
            : "Alle Anwendungen"
        }
      />
    </div>
  );
}

/** A device field: the system's choice, then the devices the browser or
 * app lists (devices.ts). */
function DeviceSelect({
  id,
  label,
  devices,
  value,
  onChange,
}: {
  id: string;
  label: string;
  devices: DeviceOption[];
  value: string;
  onChange: (value: string) => void;
}) {
  const choices = deviceChoices(devices, value);
  return (
    <Select
      id={id}
      label={label}
      value={choices.value}
      onChange={onChange}
      options={choices.options}
    />
  );
}

function Select({
  id,
  label,
  value,
  onChange,
  options,
  defaultLabel = null,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: { id: string; label: string }[];
  /** An empty first entry, for "none chosen". */
  defaultLabel?: string | null;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <label htmlFor={id} className="text-sm font-medium text-lr-text">
        {label}
      </label>
      <UiSelect
        id={id}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      >
        {defaultLabel !== null && <option value="">{defaultLabel}</option>}
        {options.map((option) => (
          <option key={option.id} value={option.id}>
            {option.label}
          </option>
        ))}
      </UiSelect>
    </div>
  );
}

function Toggle({
  id,
  label,
  checked,
  onChange,
  disabled,
}: {
  id: string;
  label: string;
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <Switch
      id={id}
      label={label}
      checked={checked}
      disabled={disabled}
      onChange={(event) => onChange(event.target.checked)}
    />
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
  scale,
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
  /** Labels under the start, middle and end of the track. */
  scale?: readonly [string, string, string];
}) {
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2 text-sm">
        <label htmlFor={id} className="font-medium text-lr-text">
          {label}
        </label>
        <span className="tabular-nums text-lr-muted">{suffix}</span>
      </div>
      <input
        id={id}
        type="range"
        step={step}
        min={min}
        max={max}
        value={value}
        onChange={(event) => onChange(Number(event.target.value))}
        style={
          {
            "--lr-range-value": `${Math.round(((value - min) / (max - min)) * 100)}%`,
          } as CSSProperties
        }
        className="lr-range"
      />
      {scale ? (
        <div
          aria-hidden="true"
          className="flex justify-between text-xs tabular-nums text-lr-muted"
        >
          {scale.map((mark) => (
            <span key={mark}>{mark}</span>
          ))}
        </div>
      ) : null}
      {hint ? <p className="text-xs text-lr-muted">{hint}</p> : null}
    </div>
  );
}
