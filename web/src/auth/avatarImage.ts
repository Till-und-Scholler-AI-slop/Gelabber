/** Edge length of uploaded avatars; shown at most at 96 CSS px (×2 for HiDPI). */
export const AVATAR_SIZE = 256;
/** Uploaded banners: 3:1, shown at most ~480 CSS px wide on profile cards. */
export const BANNER_WIDTH = 960;
export const BANNER_HEIGHT = 320;
/** Refuse absurd inputs before decoding them. */
export const AVATAR_INPUT_MAX = 20 * 1024 * 1024;

/**
 * Crops the picked image to its centered square and scales it to
 * `AVATAR_SIZE`. WebP where the browser can encode it, otherwise JPEG, or
 * PNG when the image has transparency. Animated GIFs keep their first frame.
 */
export function prepareAvatar(file: Blob): Promise<Blob> {
  return prepareImage(file, AVATAR_SIZE, AVATAR_SIZE, false);
}

/**
 * Crops the picked image to its centered 3:1 strip and scales it to
 * `BANNER_WIDTH`×`BANNER_HEIGHT`. Always opaque (transparent parts become
 * black), so the fallback is JPEG and stays far below the API's 1 MiB.
 */
export function prepareBanner(file: Blob): Promise<Blob> {
  return prepareImage(file, BANNER_WIDTH, BANNER_HEIGHT, true);
}

/** The centered source rectangle with the target's aspect ratio. */
export function centeredCrop(
  width: number,
  height: number,
  targetWidth: number,
  targetHeight: number,
): { x: number; y: number; width: number; height: number } {
  const ratio = targetWidth / targetHeight;
  const cropWidth = Math.min(width, height * ratio);
  const cropHeight = cropWidth / ratio;
  return {
    x: (width - cropWidth) / 2,
    y: (height - cropHeight) / 2,
    width: cropWidth,
    height: cropHeight,
  };
}

async function prepareImage(
  file: Blob,
  targetWidth: number,
  targetHeight: number,
  opaque: boolean,
): Promise<Blob> {
  if (file.size > AVATAR_INPUT_MAX) throw new AvatarImageError("too_large");
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new AvatarImageError("unreadable");
  }
  try {
    const crop = centeredCrop(
      bitmap.width,
      bitmap.height,
      targetWidth,
      targetHeight,
    );
    const canvas = document.createElement("canvas");
    canvas.width = targetWidth;
    canvas.height = targetHeight;
    const context = canvas.getContext("2d");
    if (!context || crop.width === 0 || crop.height === 0)
      throw new AvatarImageError("unreadable");
    if (opaque) {
      context.fillStyle = "#000";
      context.fillRect(0, 0, targetWidth, targetHeight);
    }
    context.imageSmoothingQuality = "high";
    context.drawImage(
      bitmap,
      crop.x,
      crop.y,
      crop.width,
      crop.height,
      0,
      0,
      targetWidth,
      targetHeight,
    );
    const webp = await toBlob(canvas, "image/webp", 0.85);
    if (webp?.type === "image/webp") return webp;
    // No WebP encoder (Safari, WebKitGTK in the Linux app): lossless PNG is
    // several times larger, so only keep it where transparency needs it.
    const pixels = context.getImageData(0, 0, targetWidth, targetHeight).data;
    if (fallbackType(pixels) === "image/jpeg") {
      const jpeg = await toBlob(canvas, "image/jpeg", 0.85);
      if (jpeg?.type === "image/jpeg") return jpeg;
    }
    const png = await toBlob(canvas, "image/png");
    if (!png) throw new AvatarImageError("unreadable");
    return png;
  } finally {
    bitmap.close();
  }
}

/**
 * Format when WebP is unavailable: JPEG unless a pixel is not fully
 * opaque (RGBA bytes, as from `getImageData`).
 */
export function fallbackType(
  rgba: ArrayLike<number>,
): "image/jpeg" | "image/png" {
  for (let alpha = 3; alpha < rgba.length; alpha += 4) {
    if (rgba[alpha] !== 255) return "image/png";
  }
  return "image/jpeg";
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
