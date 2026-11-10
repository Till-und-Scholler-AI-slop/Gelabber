// The call: the voice room and its stage, tiles, the call bar and the
// voice controls in the sidebar, Live, and the call's notices and errors.
import { defineCatalog } from "../catalog.ts";

export const voice = defineCatalog(
  "voice",
  {
    percent: "{percent} %",
    "stateIcons.deafenedIn": "Taub in {channel}",
    "stateIcons.deafened": "Taub",
    "stateIcons.mutedIn": "Stumm in {channel}",
    "stateIcons.muted": "Stumm",
    "stateIcons.inChannel": "In {channel}",
    "stateIcons.inVoice": "Im Sprachkanal",
    "userVolume.sectionLabel": "Lautstärke von {name}",
    "userVolume.volume": "Lautstärke",
    "userVolume.muteLocally": "Lokal stummschalten",
    "userVolume.reset": "Zurücksetzen",
    "userVolume.hint": "Nur für dich hörbar. Live-Ton bleibt unverändert.",
    "userVolume.locallyMuted": "Lokal stummgeschaltet",
  },
  {
    percent: "{percent}%",
    "stateIcons.deafenedIn": "Deafened in {channel}",
    "stateIcons.deafened": "Deafened",
    "stateIcons.mutedIn": "Muted in {channel}",
    "stateIcons.muted": "Muted",
    "stateIcons.inChannel": "In {channel}",
    "stateIcons.inVoice": "In a voice channel",
    "userVolume.sectionLabel": "Volume of {name}",
    "userVolume.volume": "Volume",
    "userVolume.muteLocally": "Mute for me",
    "userVolume.reset": "Reset",
    "userVolume.hint": "Only you hear this. Live audio stays as it is.",
    "userVolume.locallyMuted": "Muted for me",
  },
);
