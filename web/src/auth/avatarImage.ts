/** Edge length of uploaded avatars; shown at most at 96 CSS px (×2 for HiDPI). */
export const AVATAR_SIZE = 256;
/** Refuse absurd inputs before decoding them. */
export const AVATAR_INPUT_MAX = 20 * 1024 * 1024;

/**
 * Crops the picked image to its centered square and scales it to
 * `AVATAR_SIZE`. WebP where the browser can encode it, PNG otherwise
 * (Safari). Animated GIFs keep their first frame.
 */
export async function prepareAvatar(file: Blob): Promise<Blob> {
  if (file.size > AVATAR_INPUT_MAX) throw new AvatarImageError("too_large");
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new AvatarImageError("unreadable");
  }
  try {
    const side = Math.min(bitmap.width, bitmap.height);
    const canvas = document.createElement("canvas");
    canvas.width = AVATAR_SIZE;
    canvas.height = AVATAR_SIZE;
    const context = canvas.getContext("2d");
    if (!context || side === 0) throw new AvatarImageError("unreadable");
    context.imageSmoothingQuality = "high";
    context.drawImage(
      bitmap,
      (bitmap.width - side) / 2,
      (bitmap.height - side) / 2,
      side,
      side,
      0,
      0,
      AVATAR_SIZE,
      AVATAR_SIZE,
    );
    const webp = await toBlob(canvas, "image/webp", 0.85);
    if (webp?.type === "image/webp") return webp;
    const png = await toBlob(canvas, "image/png");
    if (!png) throw new AvatarImageError("unreadable");
    return png;
  } finally {
    bitmap.close();
  }
}

function toBlob(
  canvas: HTMLCanvasElement,
  type: string,
  quality?: number,
): Promise<Blob | null> {
  return new Promise((resolve) => canvas.toBlob(resolve, type, quality));
}

export class AvatarImageError extends Error {
  constructor(readonly reason: "too_large" | "unreadable") {
    super(reason);
    this.name = "AvatarImageError";
  }
}

export function avatarImageMessage(error: AvatarImageError): string {
  return error.reason === "too_large"
    ? "Das Bild ist zu groß (höchstens 20 MB)."
    : "Dieses Bild kann der Browser nicht lesen. PNG, JPEG, WebP oder GIF gehen.";
}
