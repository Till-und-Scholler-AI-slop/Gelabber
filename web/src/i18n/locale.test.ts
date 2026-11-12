import { afterEach, describe, expect, it, vi } from "vitest";

import {
  currentLocale,
  detectLocale,
  intlLocale,
  LOCALE_STORAGE_KEY,
  setLocale,
  storedLocale,
} from "./locale.ts";

function fakeStorage(throws = false) {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key: string) => {
      if (throws) throw new Error("blocked");
      return values.get(key) ?? null;
    },
    setItem: (key: string, value: string) => {
      if (throws) throw new Error("blocked");
      values.set(key, value);
    },
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
  setLocale("de", { persist: false });
});

describe("detectLocale", () => {
  it.each([
    [["de-AT"], "de"],
    [["de"], "de"],
    [["en-US"], "en"],
    [["fr-FR", "en-US"], "en"],
    [["fr-FR", "de-CH", "en"], "de"],
    [["fr"], "en"],
    [[], "en"],
  ] as const)("%j → %s", (languages, locale) => {
    expect(detectLocale(languages)).toBe(locale);
  });
});

describe("setLocale", () => {
  it("updates the store, <html lang> and the stored language", () => {
    const storage = fakeStorage();
    const html = { lang: "de" };
    vi.stubGlobal("localStorage", storage);
    vi.stubGlobal("document", { documentElement: html });
    setLocale("en");
    expect(currentLocale()).toBe("en");
    expect(html.lang).toBe("en");
    expect(storage.values.get(LOCALE_STORAGE_KEY)).toBe("en");
    expect(storedLocale()).toBe("en");
  });

  it("leaves the storage alone without persist", () => {
    const storage = fakeStorage();
    vi.stubGlobal("localStorage", storage);
    vi.stubGlobal("document", { documentElement: { lang: "de" } });
    setLocale("en", { persist: false });
    expect(currentLocale()).toBe("en");
    expect(storage.values.size).toBe(0);
  });

  it("still switches when the storage throws", () => {
    const html = { lang: "de" };
    vi.stubGlobal("localStorage", fakeStorage(true));
    vi.stubGlobal("document", { documentElement: html });
    expect(() => setLocale("en")).not.toThrow();
    expect(currentLocale()).toBe("en");
    expect(html.lang).toBe("en");
    expect(storedLocale()).toBeNull();
  });
});

describe("intlLocale", () => {
  it("formats German as de-DE and English as en-GB", () => {
    expect(intlLocale("de")).toBe("de-DE");
    expect(intlLocale("en")).toBe("en-GB");
  });
});
