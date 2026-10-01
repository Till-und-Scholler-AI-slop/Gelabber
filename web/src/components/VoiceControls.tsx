// Mute / deafen / camera / screen / leave. The click flips local state first.

import {
  CameraIcon,
  CameraOffIcon,
  GearIcon,
  HeadsetIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicIcon,
  MicOffIcon,
  ScreenIcon,
} from "./Icons.tsx";
import {
  leaveVoice,
  toggleCamera,
  toggleDeafen,
  toggleGoLive,
  toggleMute,
  toggleShare,
  useVoice,
} from "../voice/session.ts";
import { useMediaSettings } from "../voice/settings.ts";
import "../voice/room.css";

export function VoiceControls({
  compact = false,
  canGoLive = false,
}: {
  compact?: boolean;
  canGoLive?: boolean;
}) {
  const muted = useVoice((s) => s.muted);
  const deafened = useVoice((s) => s.deafened);
  const camera = useVoice((s) => s.camera);
  const sharing = useVoice((s) => s.sharing);
  const live = useVoice((s) => s.live);
  const volume = useMediaSettings((s) => s.outputVolume);
  const patch = useMediaSettings((s) => s.patch);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const micOff = muted || deafened;
  const btn = `voice-control${compact ? " voice-control-compact" : ""}`;
  const liveBtn = `${btn}${live ? " voice-control-live" : ""}`;
  const leave = `${btn} voice-control-leave`;

  return (
    <div
      className={`voice-controls${compact ? " voice-controls-compact" : ""}`}
    >
      <button
        type="button"
        aria-pressed={micOff}
        aria-label={micOff ? "Mikrofon an" : "Mikrofon aus"}
        title={micOff ? "Mikrofon an" : "Mikrofon aus"}
        onClick={() => toggleMute()}
        className={btn}
      >
        {micOff ? <MicOffIcon size={16} /> : <MicIcon size={16} />}
        {compact ? null : (
          <span className="ml-1.5">{micOff ? "Stumm" : "Mikrofon"}</span>
        )}
      </button>
      <button
        type="button"
        aria-pressed={deafened}
        aria-label={deafened ? "Hören" : "Taub stellen"}
        title={deafened ? "Hören" : "Taub stellen"}
        onClick={() => toggleDeafen()}
        className={btn}
      >
        {deafened ? <HeadsetOffIcon size={16} /> : <HeadsetIcon size={16} />}
        {compact ? null : (
          <span className="ml-1.5">{deafened ? "Taub" : "Hören"}</span>
        )}
      </button>
      <button
        type="button"
        aria-pressed={camera}
        aria-label={camera ? "Kamera aus" : "Kamera an"}
        title={camera ? "Kamera aus" : "Kamera an"}
        onClick={() => toggleCamera()}
        className={btn}
      >
        {camera ? <CameraOffIcon size={16} /> : <CameraIcon size={16} />}
        {compact ? null : (
          <span className="ml-1.5">{camera ? "Kamera aus" : "Kamera"}</span>
        )}
      </button>
      <button
        type="button"
        aria-pressed={sharing}
        aria-label={sharing ? "Teilen beenden" : "Bildschirm teilen"}
        title={sharing ? "Teilen beenden" : "Bildschirm teilen"}
        onClick={() => toggleShare()}
        className={btn}
      >
        <ScreenIcon size={16} />
        {compact ? null : (
          <span className="ml-1.5">{sharing ? "Stopp" : "Bildschirm"}</span>
        )}
      </button>
      {canGoLive || live ? (
        <button
          type="button"
          aria-pressed={live}
          aria-label={live ? "Live beenden" : "Go Live"}
          title={live ? "Live beenden" : "Go Live"}
          onClick={() => toggleGoLive()}
          className={liveBtn}
        >
          <LiveIcon size={16} />
          {compact ? null : (
            <span className="ml-1.5">{live ? "Live aus" : "Go Live"}</span>
          )}
        </button>
      ) : null}
      <label className="voice-playback-volume">
        <span className="sr-only">Wiedergabe</span>
        <input
          type="range"
          min={0}
          max={100}
          value={Math.round(volume * 100)}
          aria-label="Wiedergabe-Lautstärke"
          disabled={deafened}
          onChange={(event) =>
            patch({ outputVolume: Number(event.target.value) / 100 })
          }
          className="voice-volume-slider"
        />
      </label>
      <button
        type="button"
        aria-label="Voice-Einstellungen"
        title="Voice-Einstellungen"
        onClick={() => openSettings()}
        className={btn}
      >
        <GearIcon size={16} />
        {compact ? null : <span className="ml-1.5">Einstellungen</span>}
      </button>
      <button type="button" onClick={() => leaveVoice()} className={leave}>
        Verlassen
      </button>
    </div>
  );
}
