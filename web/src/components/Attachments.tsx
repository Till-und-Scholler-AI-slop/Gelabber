// Attachments of a message: pictures in a grid, video and audio in the
// browser's own player, text as a preview to open, every other file as a
// download (issue #255).

import {
  useContext,
  useEffect,
  useRef,
  useState,
  type SyntheticEvent,
} from "react";

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
import { DOWNLOAD_STARTED, useDownload } from "./download.ts";
import { CheckIcon, PaperclipIcon } from "./Icons.tsx";
import { imageFit, knownImageSize, rememberImageSize } from "./imageBox.ts";
import { reloadMedia } from "./mediaReload.ts";
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
            <AttachmentImage
              key={attachment.id}
              attachment={attachment}
              fit={images.length === 1}
            />
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

/** "Herunterladen", which says so once the download started (#391). */
export function DownloadLink({
  href,
  filename,
}: {
  href: string;
  filename: string;
}) {
  const { started, link } = useDownload();
  return (
    <a href={href} download={filename} {...link}>
      {started ? (
        <span role="status">{DOWNLOAD_STARTED}</span>
      ) : (
        "Herunterladen"
      )}
    </a>
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

/** One picture; `fit`: its box is the picture's own size (imageBox.ts),
 *  not a grid cell. */
function AttachmentImage({
  attachment,
  fit,
}: {
  attachment: Attachment;
  fit: boolean;
}) {
  const openImage = useContext(OpenImage);
  const [failed, setFailed] = useState(false);
  // A new key mounts a new <img>, which asks the API again.
  const [attempt, setAttempt] = useState(0);
  const [natural, setNatural] = useState(() => knownImageSize(attachment));
  // Size known: the picture's own box, reserved before it arrives, so
  // loading does not move the list.
  const sized =
    fit && natural
      ? {
          width: imageFit(natural).width,
          aspectRatio: `${natural.width} / ${natural.height}`,
        }
      : undefined;
  const src = attachment.preview_url ?? attachmentUrl(attachment.id);
  if (failed)
    return (
      <AttachmentImageFailed
        attachment={attachment}
        onRetry={() => {
          setFailed(false);
          setAttempt((n) => n + 1);
        }}
      />
    );
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
      className={sized ? "lr-attachment-image is-sized" : "lr-attachment-image"}
    >
      <img
        key={attempt}
        src={src}
        alt={attachment.filename}
        style={sized}
        decoding="async"
        loading="lazy"
        onLoad={(event) => {
          const { naturalWidth: width, naturalHeight: height } =
            event.currentTarget;
          if (width <= 0 || height <= 0) return;
          rememberImageSize(attachment, { width, height });
          setNatural((size) =>
            size?.width === width && size.height === height
              ? size
              : { width, height },
          );
        }}
        // A picture that cannot load (storage unreachable, link expired,
        // file gone) says so instead of leaving an empty box (#348).
        onError={() => setFailed(true)}
      />
      <UploadState attachment={attachment} />
    </a>
  );
}

/** The box of a picture that did not load: what failed, a retry, and the
 *  file itself, which may still arrive as a download. Same height as a
 *  picture still loading, so the list does not move. */
export function AttachmentImageFailed({
  attachment,
  onRetry,
}: {
  attachment: Attachment;
  onRetry: () => void;
}) {
  return (
    <div className="lr-attachment-image is-failed" role="group">
      <p className="lr-attachment-image-error">
        <PaperclipIcon size={15} />
        <span className="lr-attachment-image-name">{attachment.filename}</span>
        <span role="status">Bild konnte nicht geladen werden.</span>
        <span className="lr-attachment-image-actions">
          <button type="button" onClick={onRetry}>
            Erneut laden
          </button>
          {isPendingId(attachment.id) ? null : (
            <DownloadLink
              href={attachmentDownloadUrl(attachment.id)}
              filename={attachment.filename}
            />
          )}
        </span>
      </p>
    </div>
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
  const lastReload = useRef(-Infinity);
  const download = useDownload();
  const onError = (event: SyntheticEvent<HTMLMediaElement>) => {
    const now = Date.now();
    if (reloadMedia(event.currentTarget, lastReload.current, now))
      lastReload.current = now;
    else setFailed(true);
  };
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
          onError={onError}
          aria-label={attachment.filename}
        />
      ) : (
        <audio
          src={src}
          controls
          preload="metadata"
          onError={onError}
          aria-label={attachment.filename}
        />
      )}
      <figcaption>
        <a
          href={attachmentDownloadUrl(attachment.id)}
          download={attachment.filename}
          {...download.link}
        >
          {attachment.filename}
        </a>
        {download.started ? (
          <span role="status">{DOWNLOAD_STARTED}</span>
        ) : (
          <span>{formatSize(attachment.size)}</span>
        )}
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
          <DownloadLink
            href={attachmentDownloadUrl(attachment.id)}
            filename={attachment.filename}
          />
        </p>
      ) : text === null ? (
        <p className="lr-attachment-text-state">Vorschau wird geladen…</p>
      ) : (
        <>
          <pre>
            <code>{text}</code>
          </pre>
          <DownloadLink
            href={attachmentDownloadUrl(attachment.id)}
            filename={attachment.filename}
          />
        </>
      )}
    </details>
  );
}

function AttachmentFile({ attachment }: { attachment: Attachment }) {
  const { started, link } = useDownload();
  return (
    <a
      // Saved, never opened: an installed app has no back button to
      // return from a file, and after MinIO's redirect the browser
      // ignores `download` and would leave the app (and a call).
      href={attachmentDownloadUrl(attachment.id)}
      download={attachment.filename}
      className="lr-attachment-file"
      {...link}
    >
      {started ? <CheckIcon size={15} /> : <PaperclipIcon size={15} />}
      <span>{attachment.filename}</span>
      {started ? (
        <span role="status">{DOWNLOAD_STARTED}</span>
      ) : (
        <span>{formatSize(attachment.size)}</span>
      )}
      <UploadState attachment={attachment} />
    </a>
  );
}
