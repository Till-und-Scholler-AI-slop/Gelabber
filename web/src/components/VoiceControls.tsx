// Mute / deafen / camera / screen / stream sound / Go Live / settings as icon
// buttons. The click flips local state first. Volumes live in the voice
// settings dialog; leaving is the session's own button.
//
// Camera, screen and Go Live start only where this client can capture
// (voice/capabilities.ts). Stopping is always offered. Whether a share
// carries sound is decided here, next to it, where a share can carry any.

import type { ReactNode } from "react";

import {
  CameraIcon,
  CameraOffIcon,
  GearIcon,
  HeadsetIcon,
  HeadsetOffIcon,
  LiveIcon,
  MicIcon,
  MicOffIcon,
  MusicIcon,
  ScreenIcon,
} from "./Icons.tsx";
import {
  toggleCamera,
  toggleDeafen,
  toggleGoLive,
  toggleMute,
  toggleShare,
  useVoice,
} from "../voice/session.ts";
import { useCapabilities } from "../voice/capabilities.ts";
import {
  sharesSourceAudio,
  sourceAudioChoice,
  useMediaSettings,
} from "../voice/settings.ts";
import { notify } from "./toasts.ts";
import "../voice/room.css";

function VoiceAudioToggles({ className }: { className: string }) {
  const muted = useVoice((s) => s.muted);
  const deafened = useVoice((s) => s.deafened);
  const micOff = muted || deafened;
  return (
    <>
      <button
        type="button"
        aria-pressed={micOff}
        aria-label={micOff ? "Mikrofon an" : "Mikrofon aus"}
        title={micOff ? "Mikrofon an" : "Mikrofon aus"}
        onClick={() => toggleMute()}
        className={`${className}${micOff ? " is-off" : ""}`}
      >
        {micOff ? <MicOffIcon size={17} /> : <MicIcon size={17} />}
      </button>
      <button
        type="button"
        aria-pressed={deafened}
        aria-label={deafened ? "Hören" : "Taub stellen"}
        title={deafened ? "Hören" : "Taub stellen"}
        onClick={() => toggleDeafen()}
        className={`${className}${deafened ? " is-off" : ""}`}
      >
        {deafened ? <HeadsetOffIcon size={17} /> : <HeadsetIcon size={17} />}
      </button>
    </>
  );
}

/** A capture the desktop app does not have: the control stays in its place,
 * dimmed, and says why on hover and on press. */
function Unavailable({
  label,
  reason,
  className,
  children,
}: {
  label: string;
  reason: string;
  className: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-disabled="true"
      aria-label={label}
      title={reason}
      onClick={() => notify(reason)}
      className={`${className} opacity-50`}
    >
      {children}
    </button>
  );
}

export function VoiceControls({ canGoLive = false }: { canGoLive?: boolean }) {
  const camera = useVoice((s) => s.camera);
  const sharing = useVoice((s) => s.sharing);
  const live = useVoice((s) => s.live);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
  const patch = useMediaSettings((s) => s.patch);
  const capable = useCapabilities();
  const sound = sharesSourceAudio({ sourceAudioShare });
  const btn = "voice-control voice-control-compact";

  return (
    <div className="voice-controls voice-controls-compact">
      <VoiceAudioToggles className={btn} />
      {camera || capable.camera ? (
        <button
          type="button"
          aria-pressed={camera}
          aria-label={camera ? "Kamera aus" : "Kamera an"}
          title={camera ? "Kamera aus" : "Kamera an"}
          onClick={() => toggleCamera()}
          className={btn}
        >
          {camera ? <CameraOffIcon size={17} /> : <CameraIcon size={17} />}
        </button>
      ) : capable.desktop ? (
        <Unavailable
          label="Kamera an"
          reason="Diese Desktop-App kann keine Kamera nutzen."
          className={btn}
        >
          <CameraIcon size={17} />
        </Unavailable>
      ) : null}
      {sharing || capable.screen ? (
        <button
          type="button"
          aria-pressed={sharing}
          aria-label={sharing ? "Teilen beenden" : "Bildschirm teilen"}
          title={sharing ? "Teilen beenden" : "Bildschirm teilen"}
          onClick={() => toggleShare()}
          className={btn}
        >
          <ScreenIcon size={17} />
        </button>
      ) : capable.desktop ? (
        <Unavailable
          label="Bildschirm teilen"
          reason="Diese Desktop-App kann den Bildschirm nicht teilen."
          className={btn}
        >
          <ScreenIcon size={17} />
        </Unavailable>
      ) : null}
      {capable.appAudio ? (
        <button
          type="button"
          aria-pressed={sound}
          aria-label={
            sound ? "Stream-Ton nicht mehr teilen" : "Stream-Ton teilen"
          }
          title={sound ? "Stream-Ton nicht mehr teilen" : "Stream-Ton teilen"}
          onClick={() => patch({ sourceAudioShare: sourceAudioChoice(!sound) })}
          className={btn}
        >
          <MusicIcon size={17} />
        </button>
      ) : null}
      {live || (canGoLive && capable.screen) ? (
        <button
          type="button"
          aria-pressed={live}
          aria-label={live ? "Live beenden" : "Go Live"}
          title={live ? "Live beenden" : "Go Live"}
          onClick={() => toggleGoLive()}
          className={`${btn}${live ? " voice-control-live" : ""}`}
        >
          <LiveIcon size={17} />
        </button>
      ) : canGoLive && capable.desktop ? (
        <Unavailable
          label="Go Live"
          reason="Diese Desktop-App kann kein Go Live starten."
          className={btn}
        >
          <LiveIcon size={17} />
        </Unavailable>
      ) : null}
      <button
        type="button"
        aria-label="Voice-Einstellungen"
        title="Voice-Einstellungen"
        onClick={() => openSettings()}
        className={btn}
      >
        <GearIcon size={17} />
      </button>
    </div>
  );
}
