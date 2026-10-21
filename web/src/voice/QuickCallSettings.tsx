// The gear's quick settings: devices, the two volumes and how the
// microphone opens, one tap away in a call and before it. Everything else
// stays in the full dialog, linked at the bottom.

import { useEffect, useId, useRef, useState, type CSSProperties } from "react";

import { GearIcon } from "../components/Icons.tsx";
import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Popover } from "../components/ui/Popover.tsx";
import { Select } from "../components/ui/Select.tsx";
import { canChooseSpeaker, useCapabilities } from "./capabilities.ts";
import {
  deviceChoices,
  needsDeviceLabels,
  useMediaDevices,
} from "./devices.ts";
import { createMicrophoneSession } from "./microphone.ts";
import { isDesktopApp } from "./native/bridge.ts";
import type { VoiceInputMode } from "./pushToTalk.ts";
import { useVoice, voiceMicrophones } from "./session.ts";
import { MAX_PLAYBACK_VOLUME, useMediaSettings } from "./settings.ts";
import "./room.css";

/** The gear that opens the quick settings. */
export function QuickCallSettingsButton({
  className,
  variant = "ghost",
}: {
  className?: string;
  variant?: "ghost" | "secondary";
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <IconButton
        ref={anchor}
        variant={variant}
        label="Voice-Einstellungen"
        aria-haspopup="dialog"
        aria-expanded={open}
        icon={<GearIcon size={17} />}
        onClick={() => setOpen((value) => !value)}
        className={className}
      />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label="Voice-Einstellungen"
        placement="top-end"
        className="voice-quick-settings"
      >
        <QuickCallSettings onMore={() => setOpen(false)} />
      </Popover>
    </>
  );
}

export function QuickCallSettings({ onMore }: { onMore: () => void }) {
  const ids = useId();
  const settings = useMediaSettings();
  const openDialog = useMediaSettings((s) => s.openDialog);
  const capable = useCapabilities();
  const joined = useVoice((s) => s.status === "joined");
  const micLive = useVoice((s) => s.mic === "live" || s.mic === "ptt-idle");
  const { devices, reveal } = useMediaDevices();
  const desktop = isDesktopApp();
  const microphone = deviceChoices(devices.audioinput, settings.audioInputId);
  const speaker = deviceChoices(devices.audiooutput, settings.audioOutputId);
  const camera = deviceChoices(devices.videoinput, settings.videoInputId);
  const [denied, setDenied] = useState(false);
  return (
    <div className="voice-quick">
      <div className="voice-quick-field">
        <label htmlFor={`${ids}-mic`}>Mikrofon</label>
        <Select
          id={`${ids}-mic`}
          value={microphone.value}
          onChange={(event) =>
            settings.patch({ audioInputId: event.target.value })
          }
        >
          {microphone.options.map((option) => (
            <option key={option.id} value={option.id}>
              {option.label}
            </option>
          ))}
        </Select>
        {joined && micLive && !desktop ? <SendLevel /> : null}
        {!desktop && needsDeviceLabels(devices.audioinput) ? (
          <div className="voice-quick-reveal">
            <span>
              {denied
                ? "Kein Zugriff aufs Mikrofon."
                : "Namen zeigt der Browser erst nach deiner Erlaubnis."}
            </span>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void reveal("audio").then((ok) => setDenied(!ok))}
            >
              Gerätenamen anzeigen
            </Button>
          </div>
        ) : null}
      </div>
      {canChooseSpeaker() ? (
        <div className="voice-quick-field">
          <label htmlFor={`${ids}-speaker`}>Lautsprecher</label>
          <Select
            id={`${ids}-speaker`}
            value={speaker.value}
            onChange={(event) =>
              settings.patch({ audioOutputId: event.target.value })
            }
          >
            {speaker.options.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </Select>
        </div>
      ) : null}
      {capable.camera ? (
        <div className="voice-quick-field">
          <label htmlFor={`${ids}-camera`}>Kamera</label>
          <Select
            id={`${ids}-camera`}
            value={camera.value}
            onChange={(event) =>
              settings.patch({ videoInputId: event.target.value })
            }
          >
            {camera.options.map((option) => (
              <option key={option.id} value={option.id}>
                {option.label}
              </option>
            ))}
          </Select>
          {!desktop && needsDeviceLabels(devices.videoinput) ? (
            <Button
              size="sm"
              variant="ghost"
              className="voice-quick-camera-names"
              onClick={() => void reveal("video")}
            >
              Kameranamen anzeigen
            </Button>
          ) : null}
        </div>
      ) : null}
      <Volume
        id={`${ids}-voices`}
        label="Gespräche"
        value={settings.outputVolume}
        onChange={(outputVolume) => settings.patch({ outputVolume })}
      />
      <Volume
        id={`${ids}-stream`}
        label="Stream-Ton"
        value={settings.sourceAudioVolume}
        disabled={settings.sourceAudioMuted}
        onChange={(sourceAudioVolume) => settings.patch({ sourceAudioVolume })}
      />
      <ChipGroup<VoiceInputMode>
        label="Sprachübertragung"
        className="voice-quick-mode"
        options={[
          { value: "activity", label: "Sprachaktivierung" },
          { value: "ptt", label: "Push-to-Talk" },
        ]}
        value={settings.voiceInput}
        onChange={(voiceInput) => settings.patch({ voiceInput })}
      />
      {settings.voiceInput === "ptt" && !settings.pttKey ? (
        <p className="voice-quick-note">
          Noch keine Taste festgelegt: Ohne sie bleibt dein Mikrofon stumm.
        </p>
      ) : null}
      <Button
        size="sm"
        variant="ghost"
        className="voice-quick-more"
        onClick={() => {
          onMore();
          openDialog();
        }}
      >
        Alle Einstellungen …
      </Button>
    </div>
  );
}

function Volume({
  id,
  label,
  value,
  disabled = false,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  const percent = Math.round(value * 100);
  return (
    <div className="voice-quick-volume">
      <label htmlFor={id}>{label}</label>
      <output htmlFor={id}>{percent} %</output>
      <input
        id={id}
        type="range"
        min={0}
        max={MAX_PLAYBACK_VOLUME * 100}
        value={percent}
        aria-valuetext={`${percent} %`}
        disabled={disabled}
        style={
          {
            "--lr-range-value": `${Math.round((value / MAX_PLAYBACK_VOLUME) * 100)}%`,
          } as CSSProperties
        }
        onChange={(event) => onChange(Number(event.target.value) / 100)}
        className="lr-range"
      />
    </div>
  );
}

/** What the call sends right now, from a copy of the microphone track the
 * call holds: no second capture, which a phone would take the first one
 * away for. */
function SendLevel() {
  const [level, setLevel] = useState(0);
  useEffect(() => {
    const track = voiceMicrophones().sender?.track;
    if (!track || track.readyState === "ended") return;
    const copy = track.clone();
    const meter = createMicrophoneSession(
      { onState: () => {}, onLevel: setLevel },
      {
        mediaDevices: {
          getUserMedia: async () => new MediaStream([copy]),
        },
        AudioContextClass: window.AudioContext,
        requestFrame: window.requestAnimationFrame.bind(window),
        cancelFrame: window.cancelAnimationFrame.bind(window),
      },
    );
    void meter.start();
    return () => {
      meter.stop();
      copy.stop();
    };
  }, []);
  return (
    <meter
      className="voice-quick-level"
      min={0}
      max={100}
      low={8}
      value={level}
      aria-label="Pegel, den die anderen hören"
    />
  );
}
