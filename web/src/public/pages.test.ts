import { afterEach, describe, expect, it, vi } from "vitest";

import {
  NO_PUBLIC_PAGES,
  fetchPublicPages,
  legalPageUrl,
  parsePublicPages,
} from "./pages.ts";

const ALL = {
  version: 1,
  landing: true,
  legal: {
    imprint: { de: "/de/impressum", en: "/en/imprint" },
    privacy: { de: "/de/datenschutz", en: "/en/privacy" },
    terms: { de: "/de/nutzungsbedingungen", en: "/en/terms" },
  },
};

describe("parsePublicPages", () => {
  it("reads version 1 as the container writes it", () => {
    expect(parsePublicPages(ALL)).toEqual({
      landing: true,
      legal: ALL.legal,
    });
  });

  it("publishes nothing for another version or no object", () => {
    expect(parsePublicPages({ ...ALL, version: 2 })).toEqual(NO_PUBLIC_PAGES);
    expect(parsePublicPages({ ...ALL, version: "1" })).toEqual(NO_PUBLIC_PAGES);
    expect(parsePublicPages(null)).toEqual(NO_PUBLIC_PAGES);
    expect(parsePublicPages([ALL])).toEqual(NO_PUBLIC_PAGES);
    expect(parsePublicPages("{}")).toEqual(NO_PUBLIC_PAGES);
  });

  it("keeps only the pages that are there", () => {
    expect(
      parsePublicPages({
        version: 1,
        landing: false,
        legal: { privacy: ALL.legal.privacy },
      }),
    ).toEqual({ landing: false, legal: { privacy: ALL.legal.privacy } });
    expect(parsePublicPages({ version: 1 })).toEqual(NO_PUBLIC_PAGES);
    expect(parsePublicPages({ version: 1, legal: [] })).toEqual(
      NO_PUBLIC_PAGES,
    );
  });

  it("takes a page with one language and drops what is not a local path", () => {
    expect(
      parsePublicPages({
        version: 1,
        landing: "true",
        legal: {
          imprint: { de: "/de/impressum" },
          privacy: { de: "https://example.org/", en: "//example.org/" },
          terms: { de: "javascript:alert(1)", en: 3 },
          other: { de: "/de/anderes", en: "/en/other" },
        },
      }),
    ).toEqual({ landing: false, legal: { imprint: { de: "/de/impressum" } } });
  });
});

describe("fetchPublicPages", () => {
  afterEach(() => vi.unstubAllGlobals());

  function answer(response: Response | Error) {
    const fetch = vi.fn(() =>
      response instanceof Error
        ? Promise.reject(response)
        : Promise.resolve(response),
    );
    vi.stubGlobal("fetch", fetch);
    return fetch;
  }

  it("asks /public-pages.json, not the API", async () => {
    const fetch = answer(new Response(JSON.stringify(ALL)));
    expect(await fetchPublicPages()).toEqual({
      landing: true,
      legal: ALL.legal,
    });
    expect(fetch).toHaveBeenCalledWith("/public-pages.json", expect.anything());
  });

  it("publishes nothing on a 404, a network error or invalid JSON", async () => {
    answer(new Response("<!doctype html>", { status: 404 }));
    expect(await fetchPublicPages()).toEqual(NO_PUBLIC_PAGES);
    answer(new TypeError("Failed to fetch"));
    expect(await fetchPublicPages()).toEqual(NO_PUBLIC_PAGES);
    answer(new Response("<!doctype html><title>Gelabber</title>"));
    expect(await fetchPublicPages()).toEqual(NO_PUBLIC_PAGES);
  });
});

describe("legalPageUrl", () => {
  it("takes the UI language, else the other one", () => {
    expect(legalPageUrl(ALL.legal.terms, "en")).toBe("/en/terms");
    expect(legalPageUrl(ALL.legal.terms, "de")).toBe("/de/nutzungsbedingungen");
    expect(legalPageUrl({ de: "/de/impressum" }, "en")).toBe("/de/impressum");
    expect(legalPageUrl({}, "de")).toBeUndefined();
  });
});
