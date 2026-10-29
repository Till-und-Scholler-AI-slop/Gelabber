import { describe, expect, it } from "vitest";

import type { Attachment } from "../messages/types.ts";
import {
  imageBoxHeight,
  imageFit,
  knownImageSize,
  rememberImageSize,
} from "./imageBox.ts";

const picture = (id: string, preview_url?: string): Attachment => ({
  id,
  filename: "bild.png",
  content_type: "image/png",
  size: 1,
  preview_url,
});

describe("picture box", () => {
  it("draws a small picture at its own size", () => {
    expect(imageFit({ width: 120, height: 40 })).toEqual({
      width: 120,
      height: 40,
    });
  });

  it("scales a wide screenshot to the width, in its shape", () => {
    const { width, height } = imageFit({ width: 972, height: 114 });
    expect(width).toBe(358);
    expect(height).toBeCloseTo(41.99, 2);
  });

  it("scales a tall or large picture to the height", () => {
    expect(imageFit({ width: 300, height: 1200 })).toEqual({
      width: 59.5,
      height: 238,
    });
    expect(imageFit({ width: 1600, height: 1200 }).height).toBe(238);
    expect(imageFit({ width: 10, height: 5000 }).height).toBeCloseTo(238);
  });

  it("keeps the full height until the size is known", () => {
    expect(imageBoxHeight(picture("unseen"))).toBe(240);
    rememberImageSize(picture("wide"), { width: 972, height: 114 });
    expect(imageBoxHeight(picture("wide"))).toBeCloseTo(43.99, 2);
    rememberImageSize(picture("dot"), { width: 1, height: 1 });
    expect(imageBoxHeight(picture("dot"))).toBe(24);
  });

  it("knows a sent picture by its local preview after the server's id", () => {
    rememberImageSize(picture("tmp:1", "blob:a"), { width: 50, height: 20 });
    expect(knownImageSize(picture("server-id", "blob:a"))).toEqual({
      width: 50,
      height: 20,
    });
    expect(knownImageSize(picture("server-id"))).toBeUndefined();
  });
});
