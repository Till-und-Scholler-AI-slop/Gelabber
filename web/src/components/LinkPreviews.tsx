import { useState } from "react";

import { removePreviews } from "../messages/api.ts";
import { asPreviewList, type LinkPreview } from "../messages/types.ts";
import { CloseIcon } from "./Icons.tsx";
import { notifyError } from "./toasts.ts";

/** Below this width the image is a thumbnail beside the text. */
const LARGE_IMAGE_MIN_WIDTH = 400;

/**
 * Link preview cards under a message (#254). Everything here comes from
 * the API: images from `/api/previews/…`, so the browser contacts no
 * linked site. A YouTube video loads from `youtube-nocookie.com` only once
 * the viewer presses play.
 */
export function LinkPreviews({
  messageId,
  previews,
  canRemove,
}: {
  messageId: string;
  previews: unknown;
  canRemove: boolean;
}) {
  const list = asPreviewList(previews);
  if (list.length === 0) return null;
  return (
    <div className="lr-previews">
      {list.map((preview) => (
        <PreviewCard
          key={preview.url}
          preview={preview}
          onRemove={
            canRemove
              ? () => {
                  removePreviews(messageId).catch(notifyError);
                }
              : null
          }
        />
      ))}
    </div>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

function PreviewCard({
  preview,
  onRemove,
}: {
  preview: LinkPreview;
  onRemove: (() => void) | null;
}) {
  const large =
    preview.youtube_id !== undefined ||
    (preview.image !== undefined &&
      preview.image.width >= LARGE_IMAGE_MIN_WIDTH);
  return (
    <article
      className={`lr-preview${large ? " is-large" : ""}`}
      aria-label={`Vorschau: ${preview.title ?? hostOf(preview.url)}`}
    >
      <div className="lr-preview-text">
        <span className="lr-preview-site">
          {preview.site_name ?? hostOf(preview.url)}
        </span>
        {preview.title ? (
          <a
            href={preview.url}
            target="_blank"
            rel="noopener noreferrer nofollow"
            className="lr-preview-title"
          >
            {preview.title}
          </a>
        ) : null}
        {preview.description ? (
          <p className="lr-preview-description">{preview.description}</p>
        ) : null}
      </div>
      {preview.youtube_id ? (
        <YouTubePlayer
          id={preview.youtube_id}
          title={preview.title}
          image={preview.image?.url}
        />
      ) : preview.image ? (
        <img
          className="lr-preview-image"
          src={preview.image.url}
          width={preview.image.width}
          height={preview.image.height}
          alt=""
          loading="lazy"
          decoding="async"
        />
      ) : null}
      {onRemove ? (
        <button
          type="button"
          className="lr-preview-remove"
          title="Vorschauen entfernen"
          aria-label="Vorschauen entfernen"
          onClick={onRemove}
        >
          <CloseIcon size={12} />
        </button>
      ) : null}
    </article>
  );
}

function YouTubePlayer({
  id,
  title,
  image,
}: {
  id: string;
  title?: string;
  image?: string;
}) {
  const [playing, setPlaying] = useState(false);
  if (playing)
    return (
      <div className="lr-preview-video">
        <iframe
          src={`https://www.youtube-nocookie.com/embed/${id}?autoplay=1`}
          title={title ?? "YouTube-Video"}
          allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
          allowFullScreen
          referrerPolicy="strict-origin-when-cross-origin"
        />
      </div>
    );
  return (
    <button
      type="button"
      className="lr-preview-video"
      aria-label={`Video abspielen${title ? `: ${title}` : ""}`}
      onClick={() => setPlaying(true)}
    >
      {image ? (
        <img src={image} alt="" loading="lazy" decoding="async" />
      ) : null}
      <span className="lr-preview-play" aria-hidden>
        ▶
      </span>
    </button>
  );
}
