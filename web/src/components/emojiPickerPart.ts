// The emoji picker (composer and reactions) is a chunk of its own, with
// the emoji data.

import { translate } from "../i18n/catalog.ts";
import { emoji } from "../i18n/catalogs/emoji.ts";
import { lazyPart } from "./lazyPart.tsx";

export const EmojiPickerPart = lazyPart(
  () => import("./EmojiPicker.tsx").then((module) => module.default),
  () => translate(emoji, "picker.loadFailed"),
);
