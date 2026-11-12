// Reserved-width mute/deafen slots so avatars and names never shift
// when voice state flips (issue 12).

import { useT } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";
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
  const t = useT(voice);
  const micOff = muted || deafened;
  const title = !inVoice
    ? undefined
    : deafened
      ? channelName
        ? t("stateIcons.deafenedIn", { channel: channelName })
        : t("stateIcons.deafened")
      : micOff
        ? channelName
          ? t("stateIcons.mutedIn", { channel: channelName })
          : t("stateIcons.muted")
        : channelName
          ? t("stateIcons.inChannel", { channel: channelName })
          : t("stateIcons.inVoice");

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
