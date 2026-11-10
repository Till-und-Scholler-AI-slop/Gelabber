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
import { useT, type TFunction } from "../i18n/catalog.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
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

type T = TFunction<typeof voiceSettings.de>;

/** 100 % sits in the middle of a playback slider. */
const playbackScale = (t: T) =>
  [
    t("percent", { percent: 0 }),
    t("percent", { percent: 100 }),
    t("percent", { percent: MAX_PLAYBACK_VOLUME * 100 }),
  ] as const;

type SettingsSection = "all" | "audio" | "video";

export function MediaSettingsForm({
  section = "all",
}: {
  section?: SettingsSection;
}) {
  const t = useT(voiceSettings);
  const percent = (value: number) => t("percent", { percent: value });
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
        <p className="text-sm text-lr-muted">{t("form.intro")}</p>
      )}

      {showAudio && (
        <SettingsGroup title={t("input.title")}>
          <SettingRow label={t("input.microphone")} htmlFor="audio-input">
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
                    ? t("devices.revealMicFailed")
                    : t("devices.revealMicHint")}
                </p>
                <Button
                  size="sm"
                  onClick={() =>
                    void reveal("audio").then((ok) => setRevealFailed(!ok))
                  }
                >
                  {t("devices.revealMic")}
                </Button>
              </div>
            )}
            {/* Next to the microphone, in view when the dialog opens. */}
            <div className="media-actions">
              <Button size="sm" onClick={() => setMicrophoneTest(true)}>
                {t("input.testMicrophone")}
              </Button>
            </div>
          </SettingRow>
          <SettingRow label={t("input.gain")} htmlFor="input-gain">
            <Slider
              id="input-gain"
              min={0}
              max={200}
              value={Math.round(settings.inputGain * 100)}
              suffix={percent(Math.round(settings.inputGain * 100))}
              scale={[percent(0), percent(100), percent(200)]}
              onChange={(value) => settings.patch({ inputGain: value / 100 })}
              hint={t("input.gainHint")}
            />
          </SettingRow>
          <PushToTalkSettings />
          <SettingRow label={t("input.filter")}>
            <div className="media-cards">
              {(Object.keys(processingModes) as AudioProcessingMode[]).map(
                (mode) => (
                  <RadioCard
                    key={mode}
                    name="audio-processing"
                    label={t(processingModes[mode].label)}
                    description={t(processingModes[mode].hint)}
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
        <SettingsGroup title={t("output.title")}>
          {chooseSpeaker ? (
            <SettingRow label={t("output.speaker")} htmlFor="audio-output">
              <DeviceSelect
                id="audio-output"
                devices={devices.audiooutput}
                value={settings.audioOutputId}
                onChange={(value) => settings.patch({ audioOutputId: value })}
              />
            </SettingRow>
          ) : (
            <SettingRow label={t("output.speaker")}>
              <p className="media-hint media-row-text">
                {t("output.speakerUnsupported")}
              </p>
            </SettingRow>
          )}
          <SettingRow label={t("output.calls")} htmlFor="output-volume">
            <Slider
              id="output-volume"
              min={0}
              max={PLAYBACK_SLIDER_MAX}
              value={playbackSliderPosition(settings.outputVolume)}
              suffix={percent(Math.round(settings.outputVolume * 100))}
              scale={playbackScale(t)}
              onChange={(value) =>
                settings.patch({ outputVolume: playbackSliderVolume(value) })
              }
              hint={t("output.callsHint")}
            />
          </SettingRow>
          <SettingRow
            label={t("output.liveSound")}
            htmlFor="source-audio-volume"
          >
            <Slider
              id="source-audio-volume"
              min={0}
              max={PLAYBACK_SLIDER_MAX}
              value={playbackSliderPosition(settings.sourceAudioVolume)}
              suffix={percent(Math.round(settings.sourceAudioVolume * 100))}
              scale={playbackScale(t)}
              disabled={settings.sourceAudioMuted}
              onChange={(value) =>
                settings.patch({
                  sourceAudioVolume: playbackSliderVolume(value),
                })
              }
              hint={t("output.liveSoundHint")}
            />
            <Toggle
              id="source-audio-muted"
              label={t("output.muteLiveSound")}
              checked={settings.sourceAudioMuted}
              onChange={(sourceAudioMuted) =>
                settings.patch({ sourceAudioMuted })
              }
            />
          </SettingRow>
          <SettingRow label={t("output.callSounds")} htmlFor="call-sounds">
            <SwitchLine
              id="call-sounds"
              checked={settings.callSounds}
              onChange={(callSounds) => settings.patch({ callSounds })}
            >
              {t("output.callSoundsHint")}
            </SwitchLine>
            <Toggle
              id="other-mute-sounds"
              label={t("output.otherMuteSounds")}
              checked={settings.otherMuteSounds}
              disabled={!settings.callSounds}
              onChange={(otherMuteSounds) =>
                settings.patch({ otherMuteSounds })
              }
            />
            <div className="media-field">
              <label htmlFor="call-sound-volume">
                {t("output.callSoundVolume")}
              </label>
              <Slider
                id="call-sound-volume"
                min={0}
                max={1}
                step={0.05}
                value={settings.callSoundVolume}
                suffix={percent(Math.round(settings.callSoundVolume * 100))}
                disabled={!settings.callSounds}
                onChange={(callSoundVolume) =>
                  settings.patch({ callSoundVolume })
                }
                hint={
                  chooseSpeaker
                    ? t("output.callSoundVolumeSpeakerHint")
                    : t("output.callSoundVolumeHint")
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
              {t("output.testSound")}
            </Button>
          </SettingRow>
        </SettingsGroup>
      )}
      {showAudio && (
        <AdvancedAudio expanded={section === "all"} title={t("advanced.title")}>
          <SettingRow label={t("advanced.echo")} htmlFor="aec">
            <SwitchLine
              id="aec"
              checked={settings.echoCancellation}
              onChange={(echoCancellation) =>
                settings.patch({ echoCancellation })
              }
            >
              {t("advanced.echoHint")}
            </SwitchLine>
          </SettingRow>
          {settings.processingMode === "browser" && (
            <SettingRow label={t("advanced.noise")} htmlFor="ns">
              <SwitchLine
                id="ns"
                checked={settings.noiseSuppression}
                onChange={(noiseSuppression) =>
                  settings.patch({ noiseSuppression })
                }
              >
                {t("advanced.noiseHint")}
              </SwitchLine>
            </SettingRow>
          )}
          {settings.processingMode !== "original" && (
            <SettingRow label={t("advanced.autoGain")} htmlFor="agc">
              <SwitchLine
                id="agc"
                checked={settings.autoGainControl}
                onChange={(autoGainControl) =>
                  settings.patch({ autoGainControl })
                }
              >
                {t("advanced.autoGainHint")}
              </SwitchLine>
            </SettingRow>
          )}
        </AdvancedAudio>
      )}

      {showVideo && (
        <SettingsGroup
          title={t("camera.title")}
          badge={profileSummary(t, settings.cameraProfile)}
        >
          <SettingRow label={t("camera.device")} htmlFor="video-input">
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
                    ? t("devices.revealCameraFailed")
                    : t("devices.revealCameraHint")}
                </p>
                <Button
                  size="sm"
                  onClick={() =>
                    void reveal("video").then((ok) =>
                      setCameraRevealFailed(!ok),
                    )
                  }
                >
                  {t("devices.revealCamera")}
                </Button>
              </div>
            )}
          </SettingRow>
          <SettingRow label={t("quality.label")}>
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
          title={t("live.title")}
          badge={profileSummary(t, settings.screenProfile)}
        >
          <SettingRow label={t("quality.label")}>
            <StreamProfileFields
              kind="screen"
              name="screen-stream-profile"
              value={settings.screenProfile}
              apply={settings.screenProfileApply}
              onChange={(screenProfile) => settings.patch({ screenProfile })}
            />
            <p className="media-hint">{t("live.qualityHint")}</p>
          </SettingRow>
          {/* Only where a Live can carry sound; the same switch sits in
              the Live menu of the call controls. */}
          {capable.appAudio && (
            <section
              className="stream-source-audio"
              aria-label={t("live.soundLabel")}
            >
              <SettingRow
                label={t("live.shareSound")}
                htmlFor="share-source-audio"
              >
                <SwitchLine
                  id="share-source-audio"
                  checked={sharesSourceAudio(settings)}
                  onChange={(on) =>
                    settings.patch({ sourceAudioShare: sourceAudioChoice(on) })
                  }
                >
                  {t("live.shareSoundHint")}
                </SwitchLine>
              </SettingRow>
            </section>
          )}
        </SettingsGroup>
      )}
      {showVideo && (
        <SettingsGroup
          title={t("upload.title")}
          badge={
            videoLimit
              ? formatVideoBitrate(videoLimit)
              : t("upload.noLimitBadge")
          }
        >
          <SettingRow label={t("upload.economy")} htmlFor="economy-mode">
            <SwitchLine
              id="economy-mode"
              checked={settings.economyMode}
              onChange={(economyMode) =>
                settings.patch({ economyMode, quality: "normal" })
              }
            >
              {settings.economyMode
                ? t("upload.economyOn", {
                    voice: audioLimit! / 1000,
                    video: formatVideoBitrate(videoLimit!),
                  })
                : t("upload.economyOff", {
                    voice: AUDIO_QUALITY.normal.bitrate / 1000,
                    video: formatVideoBitrate(VIDEO_SEND_BUDGET),
                  })}
            </SwitchLine>
          </SettingRow>
          <SettingRow label={t("upload.videoLimit")}>
            <div className="stream-upload-mode">
              <label>
                <input
                  type="radio"
                  className="ui-check"
                  name="video-upload-mode"
                  checked={settings.videoUploadLimit === 0}
                  onChange={() => settings.patch({ videoUploadLimit: 0 })}
                />{" "}
                {settings.economyMode
                  ? t("upload.economyLimit", {
                      limit: formatVideoBitrate(VIDEO_SEND_BUDGET),
                    })
                  : t("upload.noLimit")}
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
                {t("upload.customLimit")}
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
                <span>{t("upload.customLimitUnit")}</span>
              </div>
            )}
            <p className="media-hint">{t("upload.limitHint")}</p>
          </SettingRow>
        </SettingsGroup>
      )}
      {showVideo && (
        <p className="stream-quality-budget">{t("form.applyNote")}</p>
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
  const t = useT(voiceSettings);
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
      aria-label={t("upload.customLimitLabel")}
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
  const t = useT(voiceSettings);
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
        <small>{t("advanced.subtitle")}</small>
      </summary>
      <div className="media-disclosure-body">{children}</div>
    </details>
  );
}

/** "Automatisch" or "1080p · 60 FPS", for a group's heading. */
function profileSummary(t: T, value: StreamProfileId): string {
  if (value === "balanced") return t("quality.auto");
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
  const t = useT(voiceSettings);
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
          label={t("quality.resolution")}
          value={automatic ? "auto" : String(height)}
          onChange={(next) =>
            onChange(
              next === "auto"
                ? "balanced"
                : explicitStreamProfile(Number(next) as VideoResolution, fps),
            )
          }
          options={[
            { id: "auto", label: t("quality.resolutionAuto") },
            ...VIDEO_RESOLUTIONS.map((resolution) => ({
              id: String(resolution),
              label: resolution === 2160 ? "4K · 2160p" : `${resolution}p`,
            })),
          ]}
        />
        <Select
          id={`${name}-fps`}
          label={t("quality.frameRate")}
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
              ? [{ id: "auto", label: t("quality.frameRateAuto") }]
              : []),
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
  const t = useT(voiceSettings);
  if (apply === "live") {
    return <p className="media-hint">{t("quality.appliedLive")}</p>;
  }
  if (apply === "next") {
    return <p className="media-hint">{t("quality.appliesNext")}</p>;
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
