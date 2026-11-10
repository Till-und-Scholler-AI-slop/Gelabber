import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  defineCatalog,
  richPieces,
  T,
  translate,
  translateIn,
  type Catalog,
} from "./catalog.ts";
import { setLocale } from "./locale.ts";

const sample = defineCatalog(
  "sample",
  {
    plain: "Schließen",
    greeting: "Hallo {name}, du hast {count} Punkte.",
    people: { one: "{count} Person", other: "{count} Personen" },
    rich: "Lies die <link>Hilfe</link> für {topic}.",
  },
  {
    plain: "Close",
    greeting: "Hi {name}, you have {count} points.",
    people: { one: "{count} person", other: "{count} people" },
    rich: "Read the <link>help</link> on {topic}.",
  },
);

// The types are the parity check `npm run build` runs. Never called.
export function typeChecks() {
  defineCatalog(
    "broken",
    { a: "A", b: "B" },
    // @ts-expect-error: English lacks "b".
    { a: "A" },
  );
  defineCatalog(
    "broken",
    { a: "A" },
    // @ts-expect-error: "c" is not a German key.
    { a: "A", c: "C" },
  );
  defineCatalog(
    "broken",
    { a: { one: "1", other: "{count}" } },
    // @ts-expect-error: a plural stays a plural.
    { a: "A" },
  );
  // @ts-expect-error: the placeholder must be filled in.
  translate(sample, "greeting");
  // @ts-expect-error: a plural needs its count.
  translate(sample, "people", {});
  // @ts-expect-error: no such key.
  translate(sample, "missing");
}

afterEach(() => {
  setLocale("de", { persist: false });
  vi.restoreAllMocks();
});

describe("translate", () => {
  it("follows the interface language", () => {
    expect(translate(sample, "plain")).toBe("Schließen");
    setLocale("en", { persist: false });
    expect(translate(sample, "plain")).toBe("Close");
  });

  it("fills in placeholders, numbers in the language's format", () => {
    expect(
      translateIn("de", sample, "greeting", { name: "Lena", count: 1234 }),
    ).toBe("Hallo Lena, du hast 1.234 Punkte.");
    expect(
      translateIn("en", sample, "greeting", { name: "Lena", count: 1234 }),
    ).toBe("Hi Lena, you have 1,234 points.");
  });

  it("picks the plural form by count", () => {
    expect(translateIn("de", sample, "people", { count: 1 })).toBe("1 Person");
    expect(translateIn("de", sample, "people", { count: 0 })).toBe(
      "0 Personen",
    );
    expect(translateIn("en", sample, "people", { count: 1 })).toBe("1 person");
    expect(translateIn("en", sample, "people", { count: 3 })).toBe("3 people");
  });

  it("falls back to German for a key missing at runtime and reports it", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const partial = {
      ns: "partial",
      de: { only: "Nur Deutsch" },
      en: {},
    } as unknown as Catalog<{ only: string }>;
    expect(translateIn("en", partial, "only")).toBe("Nur Deutsch");
    expect(error).toHaveBeenCalledOnce();
  });
});

describe("rich text", () => {
  it("splits tagged pieces", () => {
    expect(richPieces("a <b>b</b> c <link>d</link>")).toEqual([
      "a ",
      { tag: "b", text: "b" },
      " c ",
      { tag: "link", text: "d" },
    ]);
    expect(richPieces("plain")).toEqual(["plain"]);
  });

  it("renders each tag through its function, placeholders filled", () => {
    const render = (
      <T
        catalog={sample}
        k="rich"
        params={{ topic: "Sprache" }}
        tags={{ link: (text) => <a href="/help">{text}</a> }}
      />
    );
    expect(renderToStaticMarkup(render)).toBe(
      'Lies die <a href="/help">Hilfe</a> für Sprache.',
    );
    setLocale("en", { persist: false });
    expect(renderToStaticMarkup(render)).toBe(
      'Read the <a href="/help">help</a> on Sprache.',
    );
  });
});
