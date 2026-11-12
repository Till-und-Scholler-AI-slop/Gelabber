// The voice room (tiles, viewer, microphone test) is a chunk of its own.
// Channel and community pages show it through this part.

import { lazyPart } from "../components/lazyPart.tsx";
import { translate } from "../i18n/catalog.ts";
import { voice } from "../i18n/catalogs/voice.ts";

export const VoiceRoomPart = lazyPart(
  () => import("./VoiceRoom.tsx").then((module) => module.VoiceRoom),
  () => translate(voice, "error.roomLoadFailed"),
);
