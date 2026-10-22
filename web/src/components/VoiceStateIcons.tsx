// Reserved-width mute/deafen slots so avatars and names never shift
// when voice state flips (issue 12).

import { HeadsetOffIcon, MicOffIcon, SpeakerIcon } from "./Icons.tsx";
import "../voice/room.css";

const SLOT = "flex size-3.5 shrink-0 items-center justify-center";

export function VoiceStateIcons({
  inVoice,
  muted,
  deafened,
  channelName,
}: {
  inVoice: boolean;
  muted: boolean;
  deafened: boolean;
  channelName?: string;
}) {
  const micOff = muted || deafened;
  const title = !inVoice
    ? undefined
    : deafened
      ? channelName
        ? `Taub in ${channelName}`
        : "Taub"
      : micOff
        ? channelName
          ? `Stumm in ${channelName}`
          : "Stumm"
        : channelName
          ? `In ${channelName}`
          : "Im Sprachkanal";

  return (
    <span
      className="voice-state-icons flex w-8 shrink-0 items-center justify-end gap-0.5"
      // A named image: a bare span's label is not read out everywhere.
      role={title ? "img" : undefined}
      title={title}
      aria-label={title}
      aria-hidden={title ? undefined : true}
    >
      <span className={SLOT}>
        {micOff ? (
          <MicOffIcon size={14} />
        ) : inVoice ? (
          <SpeakerIcon size={14} className="voice-state-icons-speaker" />
        ) : null}
      </span>
      <span className={SLOT}>
        {deafened ? <HeadsetOffIcon size={14} /> : null}
      </span>
    </span>
  );
}
