import { describe, expect, it } from "vitest";

import type { Attachment } from "../messages/types.ts";
import {
  IMAGE_SIZES_KEPT,
  imageBoxHeight,
  imageFit,
  knownImageSize,
  mosaic,
  picturesHeight,
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

  it("scales a wide screenshot to the width, at most 3:1", () => {
    const { width, height } = imageFit({ width: 972, height: 114 });
    expect(width).toBe(342);
    expect(height).toBe(114);
    // 4:1 is cropped to 3:1, then scaled to the width.
    const banner = imageFit({ width: 1200, height: 300 });
    expect(banner.width).toBe(400);
    expect(banner.height).toBeCloseTo(133.33, 2);
  });

  it("scales a tall or large picture into 400×400", () => {
    const tall = imageFit({ width: 300, height: 1200 });
    expect(tall.width).toBeCloseTo(133.33, 2);
    expect(tall.height).toBe(400);
    expect(imageFit({ width: 1600, height: 1200 })).toEqual({
      width: 400,
      height: 300,
    });
    expect(imageFit({ width: 3000, height: 4000 })).toEqual({
      width: 300,
      height: 400,
    });
    // A thin strip is cropped to 1:3, never enlarged.
    expect(imageFit({ width: 10, height: 5000 })).toEqual({
      width: 24,
      height: 30,
    });
  });

  it("reserves a box until the size is known, the API's first", () => {
    expect(imageBoxHeight(picture("unseen"))).toBe(300);
    rememberImageSize(picture("wide"), { width: 972, height: 114 });
    expect(imageBoxHeight(picture("wide"))).toBe(114);
    rememberImageSize(picture("dot"), { width: 1, height: 1 });
    expect(imageBoxHeight(picture("dot"))).toBe(24);
    expect(
      imageBoxHeight({ ...picture("api"), width: 4000, height: 3000 }),
    ).toBe(300);
  });

  it("knows the height of a mosaic before it loads", () => {
    const many = (n: number) =>
      Array.from({ length: n }, (_, i) => picture(`many${i}`));
    expect(picturesHeight(many(2))).toBe(204);
    expect(picturesHeight(many(3))).toBe(271);
    expect(picturesHeight(many(4))).toBe(404);
    expect(picturesHeight(many(5))).toBe(337);
    expect(picturesHeight(many(9))).toBe(271);
    expect(mosaic(9).tiles).toBe(6);
  });

  it("knows a sent picture by its local preview after the server's id", () => {
    rememberImageSize(picture("tmp:1", "blob:a"), { width: 50, height: 20 });
    expect(knownImageSize(picture("server-id", "blob:a"))).toEqual({
      width: 50,
      height: 20,
    });
    expect(knownImageSize(picture("server-id"))).toBeUndefined();
  });

  it("keeps the sizes seen latest, not every one of a long session", () => {
    const size = { width: 50, height: 20 };
    rememberImageSize(picture("first"), size);
    rememberImageSize(picture("again"), size);
    for (let n = 0; n < IMAGE_SIZES_KEPT - 2; n++)
      rememberImageSize(picture(`p${n}`), size);
    // Full, nothing lost yet.
    expect(knownImageSize(picture("first"))).toEqual(size);
    // Shown again, a picture is the latest seen.
    rememberImageSize(picture("again"), size);
    rememberImageSize(picture("one more"), size);
    expect(knownImageSize(picture("first"))).toBeUndefined();
    expect(knownImageSize(picture("again"))).toEqual(size);
    expect(knownImageSize(picture("p0"))).toEqual(size);
    expect(knownImageSize(picture("one more"))).toEqual(size);
  });

  it("counts a sent picture's local preview as one more size", () => {
    const size = { width: 30, height: 30 };
    rememberImageSize(picture("tmp:2", "blob:b"), size);
    for (let n = 0; n < IMAGE_SIZES_KEPT - 2; n++)
      rememberImageSize(picture(`q${n}`), size);
    expect(knownImageSize(picture("server-id", "blob:b"))).toEqual(size);
    rememberImageSize(picture("q-last"), size);
    // The id went first; the preview, set after it, is next.
    expect(knownImageSize(picture("tmp:2"))).toBeUndefined();
    expect(knownImageSize(picture("server-id", "blob:b"))).toEqual(size);
    rememberImageSize(picture("q-after"), size);
    expect(knownImageSize(picture("server-id", "blob:b"))).toBeUndefined();
  });
});
