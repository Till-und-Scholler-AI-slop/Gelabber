// Every catalog in ./catalogs, German against English: what the types in
// catalog.ts cannot see. A new namespace file is picked up by itself.
import { describe, expect, it } from "vitest";

import type { Catalog, Entry } from "./catalog.ts";

const modules = import.meta.glob<Record<string, unknown>>("./catalogs/*.ts", {
  eager: true,
});

const KEY = /^[a-z][a-zA-Z0-9]*(\.[a-z][a-zA-Z0-9]*)*$/;

const texts = (entry: Entry) =>
  typeof entry === "string" ? [entry] : [entry.one, entry.other];
const placeholders = (entry: Entry) =>
  [
    ...new Set(
      texts(entry).flatMap((text) =>
        [...text.matchAll(/\{([^{}]*)\}/g)].map((match) => match[1]),
      ),
    ),
  ].sort();
const tags = (entry: Entry) =>
  texts(entry)
    .flatMap((text) => [...text.matchAll(/<\/?[^<>]*>/g)].map((m) => m[0]))
    .sort();

const catalogs = Object.entries(modules).map(([path, module]) => {
  const file = path.replace(/^\.\/catalogs\/|\.ts$/g, "");
  return { file, catalog: module[file] as Catalog | undefined };
});

describe("string catalogs", () => {
  it("are there", () => {
    expect(catalogs.map(({ file }) => file)).toEqual(
      expect.arrayContaining(["common", "language"]),
    );
  });

  describe.each(catalogs)("$file", ({ file, catalog }) => {
    it("exports the catalog under the file's name, with that ns", () => {
      expect(catalog?.ns).toBe(file);
    });

    it("has the same keys in German and English, all well-formed", () => {
      const de = Object.keys(catalog!.de);
      expect(Object.keys(catalog!.en).sort()).toEqual([...de].sort());
      for (const key of de) expect(key).toMatch(KEY);
    });

    it("has the same placeholders, tags and plural forms in both", () => {
      for (const [key, de] of Object.entries(catalog!.de)) {
        const en = (catalog!.en as Record<string, Entry>)[key];
        expect(typeof en, key).toBe(typeof de);
        if (typeof de !== "string") {
          expect(Object.keys(de).sort(), key).toEqual(["one", "other"]);
          expect(Object.keys(en).sort(), key).toEqual(["one", "other"]);
        }
        expect(placeholders(en), key).toEqual(placeholders(de));
        expect(tags(en), key).toEqual(tags(de));
      }
    });

    it("has no empty text", () => {
      for (const side of [catalog!.de, catalog!.en])
        for (const [key, entry] of Object.entries(side))
          for (const text of texts(entry as Entry))
            expect(text.trim(), key).not.toBe("");
    });
  });
});
