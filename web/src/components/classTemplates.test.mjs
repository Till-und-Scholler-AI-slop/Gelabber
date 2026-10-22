// Tailwind only builds a utility it can read as a whole word in the source.
// Glued to an interpolation (`absolute inset-x-0${…}`) the word is lost, the
// rule never ships, and the element quietly falls back: the message rows
// once shrank to their text that way. A class written right before `${` must
// therefore be one of the app's own, defined in a stylesheet here.
/* global URL */
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = new URL("../", import.meta.url);
const files = readdirSync(fileURLToPath(src), { recursive: true });
const read = (file) => readFileSync(new URL(file, src), "utf8");
const css = files
  .filter((file) => file.endsWith(".css"))
  .map(read)
  .join("\n");

/** The word written right before each `${` inside a `className` template. */
function gluedClasses(source) {
  const words = [];
  for (const [, template] of source.matchAll(/className=\{`([^`]*)`\}/g)) {
    for (const [, word] of template.matchAll(/([\w:/.[\]-]+)\$\{/g))
      words.push(word);
  }
  return words;
}

describe("class names next to an interpolation", () => {
  it("finds the glued word", () => {
    expect(
      gluedClasses(
        "<div className={`absolute inset-x-0${a ? ' b' : ''}`} />" +
          "<p className={`ui-toast ui-toast--${tone}`} />",
      ),
    ).toEqual(["inset-x-0", "ui-toast--"]);
  });

  it("are the app's own classes, not Tailwind utilities", () => {
    const offenders = files
      .filter((file) => file.endsWith(".tsx") && !file.includes(".test."))
      .flatMap((file) =>
        gluedClasses(read(file))
          // A prefix such as `ui-toast--${tone}` names a family of classes,
          // which Tailwind could not build anyway.
          .filter((word) => !word.endsWith("-"))
          .filter(
            (word) =>
              !new RegExp(
                `\\.${word.replace(/[.[\]/:]/g, "\\$&")}(?![\\w-])`,
              ).test(css),
          )
          .map((word) => `${file}: ${word}`),
      );
    expect(offenders).toEqual([]);
  });
});
