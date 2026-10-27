import { afterEach, describe, expect, it, vi } from "vitest";
import { needsCheck, readMissing, samePicture } from "./emojiSupport.ts";

afterEach(() => vi.unstubAllGlobals());

const rgba = (...pixels: number[][]) => new Uint8ClampedArray(pixels.flat());

describe("needsCheck", () => {
  it("skips the old symbols that draw everywhere", () => {
    expect(needsCheck("©️")).toBe(false);
    expect(needsCheck("☺️")).toBe(false);
  });

  it("checks supplementary-plane emoji and joined sequences", () => {
    expect(needsCheck("🫨")).toBe(true);
    expect(needsCheck("😀")).toBe(true);
    expect(needsCheck("❤️‍🔥")).toBe(true);
  });
});

describe("samePicture", () => {
  it("accepts a picture that keeps its colours in both fills", () => {
    const picture = rgba([200, 120, 40, 255], [0, 0, 0, 0]);
    expect(samePicture(picture, rgba([200, 120, 40, 255], [0, 0, 0, 0]))).toBe(
      true,
    );
  });

  it("rejects a box or glyph that takes the fill", () => {
    expect(
      samePicture(rgba([255, 0, 0, 255]), rgba([0, 0, 255, 255])),
    ).toBe(false);
  });

  it("rejects nothing drawn at all", () => {
    expect(samePicture(rgba([0, 0, 0, 0]), rgba([0, 0, 0, 0]))).toBe(false);
  });
});

describe("readMissing", () => {
  const storage = (value: string | null) => ({
    getItem: () => value,
    setItem: () => undefined,
  });

  it("returns what this browser measured for this emoji list", () => {
    vi.stubGlobal("navigator", { userAgent: "Test" });
    vi.stubGlobal(
      "localStorage",
      storage(JSON.stringify({ key: "18.0 Test", missing: ["🫨", 7] })),
    );
    expect([...(readMissing("18.0") ?? [])]).toEqual(["🫨"]);
  });

  it("measures again after a browser or emoji update", () => {
    vi.stubGlobal("navigator", { userAgent: "Other" });
    vi.stubGlobal(
      "localStorage",
      storage(JSON.stringify({ key: "18.0 Test", missing: ["🫨"] })),
    );
    expect(readMissing("18.0")).toBeNull();
    vi.stubGlobal("navigator", { userAgent: "Test" });
    expect(readMissing("19.0")).toBeNull();
  });

  it("survives broken or missing storage", () => {
    vi.stubGlobal("localStorage", storage("{nope"));
    expect(readMissing("18.0")).toBeNull();
    vi.stubGlobal("localStorage", undefined);
    expect(readMissing("18.0")).toBeNull();
  });
});
