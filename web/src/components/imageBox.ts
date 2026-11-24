// The boxes pictures are drawn in (#574). The API sends each picture's
// size as shown (`width`, `height`), so a box is reserved before any byte
// arrives and loading never moves the list. A local preview, or a picture
// from an API without sizes, is measured when it loads; those sizes are
// kept for the session, the latest IMAGE_SIZES_KEPT of them.
//
// One picture: its own shape, never enlarged, at most PICTURE_BOX² and no
// narrower or taller than 1:3 (beyond that the box crops, chat.css).
// Several: a mosaic by count (`mosaic`), at most PICTURE_BOX wide.

import type { Attachment } from "../messages/types.ts";

export type ImageSize = { width: number; height: number };

/** The largest box of a single picture, and the mosaic's width. */
export const PICTURE_BOX = 400;
/** Older name of the limits, kept for the row estimate's callers. */
export const IMAGE_BOX_WIDTH = PICTURE_BOX;
export const IMAGE_BOX_HEIGHT = PICTURE_BOX;
/** Even a tiny picture stays a target to click (chat.css). */
const IMAGE_BOX_MIN = 24;
/** The most extreme shape a single box takes; beyond it, it crops. */
const SHAPE_MAX = 3;
/** Box of a picture whose size nobody knows yet (chat.css). */
export const UNKNOWN_BOX_HEIGHT = 300;
/** Tiles a mosaic shows; the last says "+N" for the rest. */
export const MOSAIC_TILES = 6;

/** How many sizes the session keeps. A picture shown again is remembered
 *  again, so what goes first is what was seen longest ago. */
export const IMAGE_SIZES_KEPT = 2000;

/** Natural sizes by attachment id and local preview URL, oldest first. */
const known = new Map<string, ImageSize>();

function keep(key: string, size: ImageSize): void {
  // Set again at the end: a Map lists its keys in the order they came.
  known.delete(key);
  known.set(key, size);
  for (const oldest of known.keys()) {
    if (known.size <= IMAGE_SIZES_KEPT) break;
    known.delete(oldest);
  }
}

/** The size as shown: the API's, else one measured this session. */
export function knownImageSize(attachment: Attachment): ImageSize | undefined {
  if (
    typeof attachment.width === "number" &&
    typeof attachment.height === "number" &&
    attachment.width > 0 &&
    attachment.height > 0
  )
    return { width: attachment.width, height: attachment.height };
  return (
    known.get(attachment.id) ??
    (attachment.preview_url ? known.get(attachment.preview_url) : undefined)
  );
}

export function rememberImageSize(
  attachment: Attachment,
  size: ImageSize,
): void {
  keep(attachment.id, size);
  if (attachment.preview_url) keep(attachment.preview_url, size);
}

/** The box a single picture is drawn in: its size scaled into
 *  PICTURE_BOX², never enlarged, its shape held within 1:3. Not rounded:
 *  the height follows the width in the box's shape (chat.css). */
export function imageFit(size: ImageSize): ImageSize {
  const shape = Math.min(
    SHAPE_MAX,
    Math.max(1 / SHAPE_MAX, size.width / size.height),
  );
  // The natural size in the clamped shape, cropped along the longer side,
  // so no side is enlarged.
  const natural =
    shape === size.width / size.height
      ? size
      : shape > 1
        ? { width: size.height * shape, height: size.height }
        : { width: size.width, height: size.width / shape };
  const scale = Math.min(
    1,
    PICTURE_BOX / natural.width,
    PICTURE_BOX / natural.height,
  );
  return {
    width: Math.max(IMAGE_BOX_MIN, natural.width * scale),
    height: Math.max(IMAGE_BOX_MIN, natural.height * scale),
  };
}

/** The mosaic for `count` pictures: its class (chat.css), its height as a
 *  share of its width, and the tiles it shows. */
export function mosaic(count: number): {
  name: string;
  ratio: number;
  tiles: number;
} {
  if (count <= 2) return { name: "is-two", ratio: 1 / 2, tiles: 2 };
  if (count === 3) return { name: "is-three", ratio: 2 / 3, tiles: 3 };
  if (count === 4) return { name: "is-four", ratio: 1, tiles: 4 };
  if (count === 5) return { name: "is-five", ratio: 5 / 6, tiles: 5 };
  return { name: "is-six", ratio: 2 / 3, tiles: MOSAIC_TILES };
}

/** How tall the pictures of a message are, as far as known, at the
 *  widest: the virtual list's estimate before the row is measured. */
export function picturesHeight(pictures: Attachment[]): number {
  if (pictures.length === 0) return 0;
  if (pictures.length === 1) return imageBoxHeight(pictures[0]!);
  // The gap is inside the grid's aspect-ratio box.
  return Math.round(PICTURE_BOX * mosaic(pictures.length).ratio);
}

/** How tall the box of a message's only picture is, as far as known. */
export function imageBoxHeight(attachment: Attachment): number {
  const size = knownImageSize(attachment);
  return size ? imageFit(size).height : UNKNOWN_BOX_HEIGHT;
}

/** The colour shown while a picture loads, when the API sent one. */
export function placeholderColour(attachment: Attachment): string | undefined {
  return attachment.placeholder_version === 1 &&
    typeof attachment.placeholder === "string" &&
    /^#[0-9a-f]{6}$/i.test(attachment.placeholder)
    ? attachment.placeholder
    : undefined;
}
