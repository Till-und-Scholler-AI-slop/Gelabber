// Mute / deafen / camera / screen / Go Live / settings as icon buttons. The
// click flips local state first. Volumes and "share audio" live in the voice
// settings dialog; leaving is the session's own button.

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
  toggleCamera,
  toggleDeafen,
  toggleGoLive,
  toggleMute,
  toggleShare,
  useVoice,
} from "../voice/session.ts";
import { useMediaSettings } from "../voice/settings.ts";
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

export function VoiceControls({ canGoLive = false }: { canGoLive?: boolean }) {
  const camera = useVoice((s) => s.camera);
  const sharing = useVoice((s) => s.sharing);
  const live = useVoice((s) => s.live);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const btn = "voice-control voice-control-compact";

  return (
    <div className="voice-controls voice-controls-compact">
      <VoiceAudioToggles className={btn} />
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
      {canGoLive || live ? (
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
