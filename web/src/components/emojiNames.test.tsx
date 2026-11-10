import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import data from "../messages/emoji-data.json";
import EmojiPicker from "./EmojiPicker.tsx";
import { loadEnglishNames } from "./emojiNames.ts";

afterEach(() => setLocale("de", { persist: false }));

describe("English emoji names", () => {
  it("name every emoji and every group of the picker's data", async () => {
    const names = await loadEnglishNames();
    for (const entry of data.emoji) {
      const [label, keywords] = names.emoji[entry.emoji] ?? [];
      expect(label, entry.emoji).toBeTruthy();
      expect(Array.isArray(keywords), entry.emoji).toBe(true);
      expect(names.groups[entry.group], entry.group).toBeTruthy();
    }
    expect(names.emoji["😀"]).toEqual([
      "grinning face",
      expect.arrayContaining(["smile"]),
    ]);
    expect(names.groups["Gesichter und Gefühle"]).toBe("Smileys & Emotion");
  });

  it("are what the picker shows in English, and German stays German", async () => {
    await loadEnglishNames();
    const german = renderToStaticMarkup(<EmojiPicker onSelect={() => {}} />);
    expect(german).toContain('title="grinsendes Gesicht"');
    expect(german).toContain(">Gesichter und Gefühle<");
    expect(german).toContain("Emoji suchen");
    setLocale("en", { persist: false });
    const english = renderToStaticMarkup(<EmojiPicker onSelect={() => {}} />);
    expect(english).toContain('title="grinning face"');
    expect(english).toContain(">Smileys &amp; Emotion<");
    // The filter keeps the German group as its value.
    expect(english).toContain('value="Gesichter und Gefühle"');
    expect(english).toContain("Search emoji");
  });
});
