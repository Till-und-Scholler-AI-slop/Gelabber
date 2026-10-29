// The box of a single picture in a message is the picture: its own size,
// never enlarged, scaled down to fit IMAGE_BOX_WIDTH × IMAGE_BOX_HEIGHT.
// The API does not know image sizes, so a picture not seen yet keeps the
// full height until it loads (chat.css). Sizes seen are kept for the
// session, the latest IMAGE_SIZES_KEPT of them: a row that leaves the
// virtual list and comes back, or a sent picture whose row the server's
// replaces, starts at its final height.

import type { Attachment } from "../messages/types.ts";

export type ImageSize = { width: number; height: number };

/** The largest box, the picture's 1 px border included (chat.css). */
export const IMAGE_BOX_WIDTH = 360;
export const IMAGE_BOX_HEIGHT = 240;
/** Even a tiny picture stays a target to click (chat.css). */
const IMAGE_BOX_MIN = 24;
const BORDER = 2;

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

export function knownImageSize(attachment: Attachment): ImageSize | undefined {
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

/** The size a picture is drawn at, border excluded. Not rounded: the
 *  height follows the width in the picture's shape (chat.css), and a
 *  rounded width could push it past the limit. */
export function imageFit(size: ImageSize): ImageSize {
  const scale = Math.min(
    1,
    (IMAGE_BOX_WIDTH - BORDER) / size.width,
    (IMAGE_BOX_HEIGHT - BORDER) / size.height,
  );
  return { width: size.width * scale, height: size.height * scale };
}

/** How tall the box of a message's only picture is, as far as known. */
export function imageBoxHeight(attachment: Attachment): number {
  const size = knownImageSize(attachment);
  return size
    ? Math.max(IMAGE_BOX_MIN, imageFit(size).height + BORDER)
    : IMAGE_BOX_HEIGHT;
}
