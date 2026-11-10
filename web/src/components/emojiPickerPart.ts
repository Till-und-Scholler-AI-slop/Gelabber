// The emoji picker (composer and reactions) is a chunk of its own, with
// the emoji data.

import { lazyPart } from "./lazyPart.tsx";

export const EmojiPickerPart = lazyPart(
  () => import("./EmojiPicker.tsx").then((module) => module.default),
  "Die Emoji-Auswahl konnte nicht geladen werden.",
);
