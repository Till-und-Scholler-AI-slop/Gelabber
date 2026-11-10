import { useAudioProcessing } from "./audioProcessing.ts";
import { canChooseSpeaker, useCapabilities } from "./capabilities.ts";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { PushToTalkSettings } from "./PushToTalkSettings.tsx";
import {
  deviceChoices,
  needsDeviceLabels,
  useMediaDevices,
} from "./devices.ts";
// Shared Voice/Video form. Used on /settings and in-call.

import { useState, type ReactNode } from "react";
import { Button } from "../components/ui/Button.tsx";
import { Input } from "../components/ui/Input.tsx";
import { RadioCard } from "../components/ui/RadioCard.tsx";
import { Select as UiSelect } from "../components/ui/Select.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import "./quality.css";
import { playCallSound } from "./callSounds.ts";
import {
  SettingRow,
  SettingsGroup,
  Slider,
  SwitchLine,
} from "./settingsLayout.tsx";

import {
  AUDIO_PROCESSING,
  AUDIO_QUALITY,
  audioBitrate,
  type AudioProcessingMode,
  type DeviceOption,
  type StreamApply,
  type StreamKind,
  type StreamProfileId,
  STREAM_PROFILES,
  VIDEO_RESOLUTIONS,
  VIDEO_FRAME_RATES,
  VIDEO_SEND_BUDGET,
  explicitStreamProfile,
  type VideoResolution,
  type VideoFrameRate,
  formatVideoBitrate,
  MAX_PLAYBACK_VOLUME,
  PLAYBACK_SLIDER_MAX,
  playbackSliderPosition,
  playbackSliderVolume,
  sharesSourceAudio,
  sourceAudioChoice,
  uploadLimitFromField,
  useMediaSettings,
  videoSendBudget,
} from "./settings.ts";

/** 100 % sits in the middle of a playback slider. */
const PLAYBACK_SCALE = [
  "0 %",
  "100 %",
  `${MAX_PLAYBACK_VOLUME * 100} %`,
] as const;

type SettingsSection = "all" | "audio" | "video";

export function MediaSettingsForm({
  section = "all",
}: {
  section?: SettingsSection;
}) {
  const showAudio = section === "all" || section === "audio";
  const showVideo = section === "all" || section === "video";
  const settings = useMediaSettings();
  const processing = useAudioProcessing();
  const audioLimit = audioBitrate(settings);
  const videoLimit = videoSendBudget(settings);
  const [microphoneTest, setMicrophoneTest] = useState(false);
  const { devices, loaded, reveal } = useMediaDevices();
  const [revealFailed, setRevealFailed] = useState(false);
  const [cameraRevealFailed, setCameraRevealFailed] = useState(false);
  const chooseSpeaker = canChooseSpeaker();
  const capable = useCapabilities();
  const processingModes = AUDIO_PROCESSING;

  return (
    <div className="media-settings-form text-left">
      {section === "all" && (
        <p className="text-sm text-lr-muted">
          Passe Mikrofon, Lautsprecher und Kamera an. Die Übertragung hat
          standardmäßig keine Bitratengrenze durch Gelabber.
        </p>
      )}

      {showAudio && (
        <SettingsGroup title="Eingabe">
          <SettingRow label="Mikrofon" htmlFor="audio-input">
            <DeviceSelect
              id="audio-input"
              devices={devices.audioinput}
              value={settings.audioInputId}
              onChange={(value) => settings.patch({ audioInputId: value })}
            />
            {loaded && needsDeviceLabels(devices.audioinput) && (
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
            {/* Next to the microphone, in view when the dialog opens. */}
            <div className="media-actions">
              <Button size="sm" onClick={() => setMicrophoneTest(true)}>
                Mikrofon testen und vergleichen
              </Button>
            </div>
          </SettingRow>
          <SettingRow label="Mikrofonverstärkung" htmlFor="input-gain">
            <Slider
              id="input-gain"
              min={0}
              max={200}
              value={Math.round(settings.inputGain * 100)}
              suffix={`${Math.round(settings.inputGain * 100)} %`}
              scale={["0 %", "100 %", "200 %"]}
              onChange={(value) => settings.patch({ inputGain: value / 100 })}
              hint={
                "100 % fügt keine Verstärkung hinzu. Andere Werte laufen über Web Audio."
              }
            />
          </SettingRow>
          <PushToTalkSettings />
          <SettingRow label="Filter">
            <div className="media-cards">
              {(Object.keys(processingModes) as AudioProcessingMode[]).map(
                (mode) => (
                  <RadioCard
                    key={mode}
                    name="audio-processing"
                    label={processingModes[mode].label}
                    description={processingModes[mode].hint}
                    checked={settings.processingMode === mode}
                    onChange={() => settings.patch({ processingMode: mode })}
                  />
                ),
              )}
            </div>
            <p role="status" className="media-hint">
              {processing.message}
            </p>
          </SettingRow>
        </SettingsGroup>
      )}
      {showAudio && (
        <SettingsGroup title="Ausgabe">
          {chooseSpeaker ? (
            <SettingRow label="Lautsprecher" htmlFor="audio-output">
              <DeviceSelect
                id="audio-output"
                devices={devices.audiooutput}
                value={settings.audioOutputId}
                onChange={(value) => settings.patch({ audioOutputId: value })}
              />
            </SettingRow>
          ) : (
            <SettingRow label="Lautsprecher">
              <p className="media-hint media-row-text">
                Dieser Browser kann den Lautsprecher nicht wählen. Der Ton folgt
                der Ausgabe deines Geräts.
              </p>
            </SettingRow>
          )}
          <SettingRow label="Gespräche" htmlFor="output-volume">
            <Slider
              id="output-volume"
              min={0}
              max={PLAYBACK_SLIDER_MAX}
              value={playbackSliderPosition(settings.outputVolume)}
              suffix={`${Math.round(settings.outputVolume * 100)} %`}
              scale={PLAYBACK_SCALE}
              onChange={(value) =>
                settings.patch({ outputVolume: playbackSliderVolume(value) })
              }
              hint={
                "Über 100 % wird der Ton digital verstärkt. Nutze dafür Kopfhörer: Über Lautsprecher können die anderen sonst ihr eigenes Echo hören."
              }
            />
          </SettingRow>
          <SettingRow label="Live-Ton" htmlFor="source-audio-volume">
            <Slider
              id="source-audio-volume"
              min={0}
              max={PLAYBACK_SLIDER_MAX}
              value={playbackSliderPosition(settings.sourceAudioVolume)}
              suffix={`${Math.round(settings.sourceAudioVolume * 100)} %`}
              scale={PLAYBACK_SCALE}
              disabled={settings.sourceAudioMuted}
              onChange={(value) =>
                settings.patch({
                  sourceAudioVolume: playbackSliderVolume(value),
                })
              }
              hint="Der Ton von Lives hat eine eigene Lautstärke. Taub schaltet auch diesen Ton aus."
            />
            <Toggle
              id="source-audio-muted"
              label="Live-Ton stummschalten"
              checked={settings.sourceAudioMuted}
              onChange={(sourceAudioMuted) =>
                settings.patch({ sourceAudioMuted })
              }
            />
          </SettingRow>
          <SettingRow label="Signaltöne" htmlFor="call-sounds">
            <SwitchLine
              id="call-sounds"
              checked={settings.callSounds}
              onChange={(callSounds) => settings.patch({ callSounds })}
            >
              Töne für Beitreten, Verlassen, Stumm, Taub, Kamera und Live.
            </SwitchLine>
            <Toggle
              id="other-mute-sounds"
              label="Töne, wenn andere sich stumm schalten"
              checked={settings.otherMuteSounds}
              disabled={!settings.callSounds}
              onChange={(otherMuteSounds) =>
                settings.patch({ otherMuteSounds })
              }
            />
            <div className="media-field">
              <label htmlFor="call-sound-volume">
                Lautstärke der Signaltöne
              </label>
              <Slider
                id="call-sound-volume"
                min={0}
                max={1}
                step={0.05}
                value={settings.callSoundVolume}
                suffix={`${Math.round(settings.callSoundVolume * 100)} %`}
                disabled={!settings.callSounds}
                onChange={(callSoundVolume) =>
                  settings.patch({ callSoundVolume })
                }
                hint={
                  chooseSpeaker
                    ? "Nutzt deinen gewählten Lautsprecher und die Wiedergabelautstärke."
                    : "Nutzt die Wiedergabelautstärke."
                }
              />
            </div>
            <Button
              size="sm"
              className="self-start"
              disabled={
                !settings.callSounds ||
                settings.callSoundVolume === 0 ||
                settings.outputVolume === 0
              }
              onClick={() => playCallSound("joinSelf")}
            >
              Testton abspielen
            </Button>
          </SettingRow>
        </SettingsGroup>
      )}
      {showAudio && (
        <AdvancedAudio
          expanded={section === "all"}
          title="Echo und Browserfilter"
        >
          <SettingRow label="Echo" htmlFor="aec">
            <SwitchLine
              id="aec"
              checked={settings.echoCancellation}
              onChange={(echoCancellation) =>
                settings.patch({ echoCancellation })
              }
            >
              Echo-Unterdrückung (AEC). Bei Lautsprechern empfohlen. Für Musik
              mit Kopfhörern kannst du sie ausschalten.
            </SwitchLine>
          </SettingRow>
          {settings.processingMode === "browser" && (
            <SettingRow label="Rauschfilter" htmlFor="ns">
              <SwitchLine
                id="ns"
                checked={settings.noiseSuppression}
                onChange={(noiseSuppression) =>
                  settings.patch({ noiseSuppression })
                }
              >
                Rauschunterdrückung des Browsers.
              </SwitchLine>
            </SettingRow>
          )}
          {settings.processingMode !== "original" && (
            <SettingRow label="Auto-Gain" htmlFor="agc">
              <SwitchLine
                id="agc"
                checked={settings.autoGainControl}
                onChange={(autoGainControl) =>
                  settings.patch({ autoGainControl })
                }
              >
                Auto-Gain des Browsers. Gleicht leise und laute Mikrofone auf
                einen hörbaren Pegel an.
              </SwitchLine>
            </SettingRow>
          )}
        </AdvancedAudio>
      )}

      {showVideo && (
        <SettingsGroup
          title="Kamera"
          badge={profileSummary(settings.cameraProfile)}
        >
          <SettingRow label="Gerät" htmlFor="video-input">
            <DeviceSelect
              id="video-input"
              devices={devices.videoinput}
              value={settings.videoInputId}
              onChange={(value) => settings.patch({ videoInputId: value })}
            />
            {loaded && needsDeviceLabels(devices.videoinput) && (
              <div className="media-settings-reveal">
                <p className="text-xs text-lr-muted">
                  {cameraRevealFailed
                    ? "Der Browser gibt die Kameranamen ohne Kamerazugriff nicht heraus."
                    : "Die Kameranamen zeigt der Browser erst, wenn du den Kamerazugriff erlaubst."}
                </p>
                <Button
                  size="sm"
                  onClick={() =>
                    void reveal("video").then((ok) =>
                      setCameraRevealFailed(!ok),
                    )
                  }
                >
                  Kameranamen anzeigen
                </Button>
              </div>
            )}
          </SettingRow>
          <SettingRow label="Qualität">
            <StreamProfileFields
              kind="camera"
              name="camera-stream-profile"
              value={settings.cameraProfile}
              apply={settings.cameraProfileApply}
              onChange={(cameraProfile) => settings.patch({ cameraProfile })}
            />
          </SettingRow>
        </SettingsGroup>
      )}
      {showVideo && (
        <SettingsGroup
          title="Live"
          badge={profileSummary(settings.screenProfile)}
        >
          <SettingRow label="Qualität">
            <StreamProfileFields
              kind="screen"
              name="screen-stream-profile"
              value={settings.screenProfile}
              apply={settings.screenProfileApply}
              onChange={(screenProfile) => settings.patch({ screenProfile })}
            />
            <p className="media-hint">
              Höhere Werte brauchen mehr Rechenleistung und Upload. Die
              erreichbare Qualität hängt von Quelle, Gerät und Verbindung ab.
            </p>
          </SettingRow>
          {/* Only where a Live can carry sound; the same switch sits in
              the Live menu of the call controls. */}
          {capable.appAudio && (
            <section className="stream-source-audio" aria-label="Ton im Live">
              <SettingRow label="Ton mitsenden" htmlFor="share-source-audio">
                <SwitchLine
                  id="share-source-audio"
                  checked={sharesSourceAudio(settings)}
                  onChange={(on) =>
                    settings.patch({ sourceAudioShare: sourceAudioChoice(on) })
                  }
                >
                  Gilt ab deinem nächsten Live; Ausschalten wirkt sofort. Wähle
                  den Ton im Browserdialog aus; je nach Browser und Quelle ist
                  nur Video verfügbar. Der Live-Ton wird als Stereo-Musik
                  übertragen, ohne Mikrofonfilter.
                </SwitchLine>
              </SettingRow>
            </section>
          )}
        </SettingsGroup>
      )}
      {showVideo && (
        <SettingsGroup
          title="Upload"
          badge={videoLimit ? formatVideoBitrate(videoLimit) : "ohne Limit"}
        >
          <SettingRow label="Sparmodus" htmlFor="economy-mode">
            <SwitchLine
              id="economy-mode"
              checked={settings.economyMode}
              onChange={(economyMode) =>
                settings.patch({ economyMode, quality: "normal" })
              }
            >
              {settings.economyMode
                ? `Ausdrückliche Obergrenzen: Sprache ${audioLimit! / 1000} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam ${formatVideoBitrate(videoLimit!)}.`
                : `Für wenig Upload: Sprache ${AUDIO_QUALITY.normal.bitrate / 1000} kbit/s, Live-Ton 128 kbit/s, Video gemeinsam ${formatVideoBitrate(VIDEO_SEND_BUDGET)}.`}
            </SwitchLine>
          </SettingRow>
          <SettingRow label="Video-Limit">
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
              <div className="stream-upload-limit">
                <UploadLimitField
                  limit={settings.videoUploadLimit}
                  onLimit={(videoUploadLimit) =>
                    settings.patch({ videoUploadLimit })
                  }
                />
                <span>Mbit/s für alle Videos zusammen</span>
              </div>
            )}
            <p className="media-hint">
              Der Browser passt die tatsächliche Bitrate an die Verbindung an.
              Das Limit ist eine Obergrenze, keine feste Datenrate.
            </p>
          </SettingRow>
        </SettingsGroup>
      )}
      {showVideo && (
        <p className="stream-quality-budget">
          Änderungen gelten beim nächsten Start und werden, soweit unterstützt,
          auch auf laufende Streams angewendet.
        </p>
      )}
      {microphoneTest && (
        <MicrophoneTest onClose={() => setMicrophoneTest(false)} />
      )}
    </div>
  );
}

/** The custom limit in Mbit/s. The text is its own while typing: an empty
 * or half-typed field ("0" on the way to "0.5") keeps the stored limit, and
 * leaving the field shows that limit again. */
function UploadLimitField({
  limit,
  onLimit,
}: {
  limit: number;
  onLimit: (limit: number) => void;
}) {
  const [text, setText] = useState(String(limit / 1_000_000));
  const [shown, setShown] = useState(limit);
  if (shown !== limit) {
    setShown(limit);
    setText(String(limit / 1_000_000));
  }
  return (
    <Input
      id="video-upload-limit"
      type="number"
      min="0.001"
      max="4294.967"
      step="any"
      aria-label="Eigenes gemeinsames Video-Limit (Mbit/s)"
      value={text}
      onChange={(event) => {
        setText(event.target.value);
        const next = uploadLimitFromField(event.target.value);
        if (next !== null) onLimit(next);
      }}
      onBlur={() => setText(String(limit / 1_000_000))}
    />
  );
}

function AdvancedAudio({
  expanded,
  title,
  children,
}: {
  expanded: boolean;
  title: string;
  children: ReactNode;
}) {
  if (expanded)
    return (
      <SettingsGroup title={title}>
        <div>{children}</div>
      </SettingsGroup>
    );
  return (
    <details className="media-disclosure">
      <summary>
        {title}
        <small>Für Musik oder Lautsprecher</small>
      </summary>
      <div className="media-disclosure-body">{children}</div>
    </details>
  );
}

/** "Automatisch" or "1080p · 60 FPS", for a group's heading. */
function profileSummary(value: StreamProfileId): string {
  if (value === "balanced") return "Automatisch";
  const { height, fps } = STREAM_PROFILES[value];
  return `${height === 2160 ? "4K" : `${height}p`} · ${fps} FPS`;
}

function StreamProfileFields({
  kind,
  name,
  value,
  apply,
  onChange,
}: {
  kind: StreamKind;
  name: string;
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
    <>
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
            { id: "auto", label: "Auto (bis 1080p)" },
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
            ...(automatic ? [{ id: "auto", label: "Auto (bis 30 FPS)" }] : []),
            ...VIDEO_FRAME_RATES.map((rate) => ({
              id: String(rate),
              label: `${rate} FPS`,
            })),
          ]}
        />
      </div>
      <ApplyNote apply={apply} />
    </>
  );
}

function ApplyNote({ apply }: { apply: StreamApply }) {
  if (apply === "live") {
    return <p className="media-hint">Auf den laufenden Stream angewendet.</p>;
  }
  if (apply === "next") {
    return (
      <p className="media-hint">
        Bitrate und FPS-Limit gelten sofort. Die Auflösung gilt ab dem nächsten
        Stream.
      </p>
    );
  }
  return null;
}

/** A device field: the system's choice, then the devices the browser
 * lists (devices.ts). Its row names it. */
function DeviceSelect({
  id,
  devices,
  value,
  onChange,
}: {
  id: string;
  devices: DeviceOption[];
  value: string;
  onChange: (value: string) => void;
}) {
  const choices = deviceChoices(devices, value);
  return (
    <Select
      id={id}
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
  /** A small label above the field; without it, the row names the field. */
  label?: string;
  value: string;
  onChange: (value: string) => void;
  options: { id: string; label: string }[];
  /** An empty first entry, for "none chosen". */
  defaultLabel?: string | null;
}) {
  const field = (
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
  );
  if (!label) return field;
  return (
    <div className="media-field">
      <label htmlFor={id}>{label}</label>
      {field}
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
