// The voice room (tiles, viewer, microphone test) is a chunk of its own.
// Channel and community pages show it through this part.

import { lazyPart } from "../components/lazyPart.tsx";

export const VoiceRoomPart = lazyPart(
  () => import("./VoiceRoom.tsx").then((module) => module.VoiceRoom),
  "Der Sprachraum konnte nicht geladen werden.",
);
