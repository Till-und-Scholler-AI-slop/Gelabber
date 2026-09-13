// Mute / deafen / leave. The click flips local state first.

import { HeadsetIcon, HeadsetOffIcon, MicIcon, MicOffIcon } from "./Icons.tsx";
import {
  leaveVoice,
  toggleDeafen,
  toggleMute,
  useVoice,
} from "../voice/session.ts";

export function VoiceControls({ compact = false }: { compact?: boolean }) {
  const muted = useVoice((s) => s.muted);
  const deafened = useVoice((s) => s.deafened);
  const micOff = muted || deafened;
  const btn = compact
    ? "inline-flex items-center rounded-md p-1.5 text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900"
    : "inline-flex items-center rounded-lg bg-neutral-200 px-3 py-2 text-sm font-medium text-neutral-800 transition hover:bg-neutral-300";
  const leave = compact
    ? "inline-flex items-center rounded-md px-2 py-1 text-xs text-neutral-600 hover:bg-neutral-100 hover:text-neutral-900"
    : "inline-flex items-center rounded-lg bg-neutral-900 px-4 py-2 text-sm font-medium text-white transition hover:bg-neutral-700";

  return (
    <div className="flex items-center justify-center gap-2">
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
      <button type="button" onClick={() => leaveVoice()} className={leave}>
        Verlassen
      </button>
    </div>
  );
}
