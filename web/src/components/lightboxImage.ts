import {
  attachmentDownloadUrl,
  attachmentUrl,
  thumbnailUrl,
} from "../messages/api.ts";
import { isPendingId } from "../messages/queries.ts";
import type { Attachment } from "../messages/types.ts";
import type { LightboxImage } from "./ImageLightbox.tsx";

/** What the viewer shows of a picture: the 800 px thumbnail first, the
 *  original once loaded. */
export function lightboxImage(attachment: Attachment): LightboxImage {
  const original = attachment.preview_url ?? attachmentUrl(attachment.id);
  const thumbed = attachment.thumbnail && !attachment.preview_url;
  const plays = attachment.animated;
  return {
    id: attachment.id,
    src: thumbed && !plays ? thumbnailUrl(attachment.id, 800) : original,
    full: thumbed && !plays ? original : undefined,
    filename: attachment.filename,
    alt: attachment.description,
    size: attachment.size,
    download: isPendingId(attachment.id)
      ? undefined
      : attachmentDownloadUrl(attachment.id),
    open: isPendingId(attachment.id) ? undefined : attachmentUrl(attachment.id),
  };
}
