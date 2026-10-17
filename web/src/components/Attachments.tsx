// Attachments of a message: pictures in a grid, video and audio in the
// browser's own player, text as a preview to open, every other file as a
// download (issue #255).

import { useContext, useEffect, useState } from "react";

import { attachmentDownloadUrl, attachmentUrl } from "../messages/api.ts";
import { isPendingId } from "../messages/queries.ts";
import {
  decodeText,
  formatSize,
  isAudioType,
  isImageType,
  isTextType,
  isVideoType,
  TEXT_PREVIEW_MAX,
} from "../messages/rules.ts";
import type { Attachment } from "../messages/types.ts";
import { asAttachmentList } from "../messages/types.ts";
import { PaperclipIcon } from "./Icons.tsx";
import { OpenImage } from "./openImage.ts";
import { isPlainClick } from "./plainClick.ts";

export function AttachmentList({ attachments }: { attachments: Attachment[] }) {
  const files = asAttachmentList(attachments);
  if (files.length === 0) return null;
  const images = files.filter((a) => isImageType(a.content_type));
  const others = files.filter((a) => !isImageType(a.content_type));
  return (
    <div className="lr-attachments">
      {images.length > 0 ? (
        <div className={images.length > 1 ? "lr-attachment-grid" : undefined}>
          {images.map((attachment) => (
            <AttachmentImage key={attachment.id} attachment={attachment} />
          ))}
        </div>
      ) : null}
      {others.map((attachment) => {
        const type = attachment.content_type;
        if (isVideoType(type) || isAudioType(type))
          return (
            <AttachmentMedia key={attachment.id} attachment={attachment} />
          );
        if (isTextType(type) && attachment.size <= TEXT_PREVIEW_MAX)
          return <AttachmentText key={attachment.id} attachment={attachment} />;
        return <AttachmentFile key={attachment.id} attachment={attachment} />;
      })}
    </div>
  );
}

function UploadState({ attachment }: { attachment: Attachment }) {
  if (!attachment.upload || attachment.upload === "done") return null;
  return (
    <span className="lr-attachment-upload" role="status">
      {attachment.upload === "uploading" ? "Wird hochgeladen…" : "Wartet"}
    </span>
  );
}

function AttachmentImage({ attachment }: { attachment: Attachment }) {
  const openImage = useContext(OpenImage);
  const src = attachment.preview_url ?? attachmentUrl(attachment.id);
  return (
    <a
      href={src}
      // The link stays for "open in new tab"; a click shows the
      // picture here, also where no new tab exists (desktop app,
      // installed web app).
      onClick={(event) => {
        if (!isPlainClick(event)) return;
        event.preventDefault();
        if (isPendingId(attachment.id)) return;
        openImage({
          id: attachment.id,
          src,
          filename: attachment.filename,
          download: attachmentDownloadUrl(attachment.id),
        });
      }}
      aria-label={`${attachment.filename} vergrößern`}
      className="lr-attachment-image"
    >
      {/* A fixed-height box: the row has its final height before the
          picture arrives, so loading never moves the list. */}
      <img
        src={src}
        alt={attachment.filename}
        decoding="async"
        loading="lazy"
      />
      <UploadState attachment={attachment} />
    </a>
  );
}

/** Whether this browser may play the type; "" means it certainly cannot
 *  (QuickTime on Linux, for one). A file it then fails on falls back too. */
function playable(kind: "video" | "audio", type: string): boolean {
  try {
    return document.createElement(kind).canPlayType(type) !== "";
  } catch {
    return false;
  }
}

function AttachmentMedia({ attachment }: { attachment: Attachment }) {
  const kind = isVideoType(attachment.content_type) ? "video" : "audio";
  const [failed, setFailed] = useState(
    () => !playable(kind, attachment.content_type),
  );
  // Still uploading: nothing to stream yet.
  if (failed || isPendingId(attachment.id))
    return <AttachmentFile attachment={attachment} />;
  const src = attachmentUrl(attachment.id);
  return (
    <figure className={`lr-attachment-media is-${kind}`}>
      {kind === "video" ? (
        <video
          src={src}
          controls
          preload="metadata"
          playsInline
          onError={() => setFailed(true)}
          aria-label={attachment.filename}
        />
      ) : (
        <audio
          src={src}
          controls
          preload="metadata"
          onError={() => setFailed(true)}
          aria-label={attachment.filename}
        />
      )}
      <figcaption>
        <a
          href={attachmentDownloadUrl(attachment.id)}
          download={attachment.filename}
        >
          {attachment.filename}
        </a>
        <span>{formatSize(attachment.size)}</span>
      </figcaption>
    </figure>
  );
}

function AttachmentText({ attachment }: { attachment: Attachment }) {
  const [open, setOpen] = useState(false);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!open || text !== null || isPendingId(attachment.id)) return;
    // A row scrolled out of the virtual list unmounts; its request stops.
    const controller = new AbortController();
    fetch(attachmentUrl(attachment.id), {
      credentials: "same-origin",
      signal: controller.signal,
    })
      .then((response) => {
        if (!response.ok) throw new Error(String(response.status));
        return response.arrayBuffer();
      })
      .then(
        (bytes) => setText(decodeText(bytes)),
        () => {
          if (!controller.signal.aborted) setError(true);
        },
      );
    return () => controller.abort();
  }, [open, text, attachment.id]);
  return (
    <details
      className="lr-attachment-text"
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary>
        <PaperclipIcon size={15} />
        <span>{attachment.filename}</span>
        <span>{formatSize(attachment.size)}</span>
        <UploadState attachment={attachment} />
      </summary>
      {error ? (
        <p className="lr-attachment-text-state">
          Vorschau nicht verfügbar.{" "}
          <a
            href={attachmentDownloadUrl(attachment.id)}
            download={attachment.filename}
          >
            Herunterladen
          </a>
        </p>
      ) : text === null ? (
        <p className="lr-attachment-text-state">Vorschau wird geladen…</p>
      ) : (
        <>
          <pre>
            <code>{text}</code>
          </pre>
          <a
            href={attachmentDownloadUrl(attachment.id)}
            download={attachment.filename}
          >
            Herunterladen
          </a>
        </>
      )}
    </details>
  );
}

function AttachmentFile({ attachment }: { attachment: Attachment }) {
  return (
    <a
      // Saved, never opened: an installed app has no back button to
      // return from a file, and after MinIO's redirect the browser
      // ignores `download` and would leave the app (and a call).
      href={attachmentDownloadUrl(attachment.id)}
      download={attachment.filename}
      className="lr-attachment-file"
    >
      <PaperclipIcon size={15} />
      <span>{attachment.filename}</span>
      <span>{formatSize(attachment.size)}</span>
      <UploadState attachment={attachment} />
    </a>
  );
}
