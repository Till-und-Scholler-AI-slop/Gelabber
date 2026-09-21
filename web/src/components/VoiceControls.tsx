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
  const btn = compact
    ? "inline-flex items-center rounded-md p-1.5 text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100"
    : "inline-flex items-center rounded-lg bg-neutral-200 dark:bg-neutral-700 px-3 py-2 text-sm font-medium text-neutral-800 dark:text-neutral-200 transition hover:bg-neutral-300 dark:hover:bg-neutral-600";
  const liveBtn = compact
    ? [
        "inline-flex items-center rounded-md p-1.5",
        live
          ? "bg-red-600 text-white hover:bg-red-700"
          : "text-red-600 dark:text-red-400 hover:bg-red-50 dark:hover:bg-red-950 hover:text-red-700 dark:hover:text-red-300",
      ].join(" ")
    : [
        "inline-flex items-center rounded-lg px-3 py-2 text-sm font-medium transition",
        live
          ? "bg-red-600 text-white hover:bg-red-700"
          : "bg-neutral-200 dark:bg-neutral-700 text-neutral-800 dark:text-neutral-200 hover:bg-neutral-300 dark:hover:bg-neutral-600",
      ].join(" ");
  const leave = compact
    ? "inline-flex items-center rounded-md px-2 py-1 text-xs text-neutral-600 dark:text-neutral-400 hover:bg-neutral-100 dark:hover:bg-neutral-800 hover:text-neutral-900 dark:hover:text-neutral-100"
    : "inline-flex items-center rounded-lg bg-neutral-900 dark:bg-neutral-700 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700 dark:hover:bg-neutral-600";

  return (
    <div
      className={
        compact
          ? "flex flex-wrap items-center justify-end gap-1"
          : "flex flex-wrap items-center justify-center gap-2"
      }
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
      {canGoLive ? (
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
      {compact ? null : (
        <label className="inline-flex items-center gap-1.5 text-sm text-neutral-600 dark:text-neutral-400">
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
            className="w-24"
          />
        </label>
      )}
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
