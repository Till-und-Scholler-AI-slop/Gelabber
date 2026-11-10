// The emoji's names and search words in the interface's language. German
// comes with the picker (emoji-data.json); English (emoji-names.en.json,
// from English CLDR) is a chunk of its own, loaded the first time the
// picker opens in English. Until it is there the picker shows German.

import { useEffect, useState } from "react";

import type { Locale } from "../i18n/locale.ts";

export type EmojiNames = {
  /** German group name (as in emoji-data.json) → its name here. */
  groups: Record<string, string>;
  /** Emoji → [label, keywords]. */
  emoji: Record<string, [string, string[]]>;
};

let english: EmojiNames | null = null;
let loading: Promise<EmojiNames> | null = null;

/** Loads the English names; a failed load is tried again next time. */
export function loadEnglishNames(): Promise<EmojiNames> {
  loading ??= import("../messages/emoji-names.en.json").then(
    (module) => (english = module.default as unknown as EmojiNames),
    (error: unknown) => {
      loading = null;
      throw error;
    },
  );
  return loading;
}

/** The names for `locale`, or null for the German ones in the data. */
export function useEmojiNames(locale: Locale): EmojiNames | null {
  const [names, setNames] = useState(english);
  useEffect(() => {
    if (locale !== "en" || names) return;
    let alive = true;
    loadEnglishNames().then(
      (loaded) => {
        if (alive) setNames(loaded);
      },
      // The German names stay; the next opening asks again.
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [locale, names]);
  return locale === "en" ? names : null;
}
