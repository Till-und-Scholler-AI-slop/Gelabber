// Attachments of a message: pictures in a mosaic, video and audio in the
// browser's own player, text as a preview to open, every other file as a
// download (issue #255).

import {
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type SyntheticEvent,
} from "react";

import {
  attachmentDownloadUrl,
  attachmentUrl,
  thumbnailUrl,
} from "../messages/api.ts";
import { isPendingId, useEditAltText } from "../messages/queries.ts";
import { takeStamp } from "../auth/scope.ts";
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
import { translate, useT } from "../i18n/catalog.ts";
import { AltTextDialog } from "./AltTextDialog.tsx";
import { notifyError } from "./toasts.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { useDownload } from "./download.ts";
import { CheckIcon, PaperclipIcon } from "./Icons.tsx";
import {
  imageFit,
  knownImageSize,
  mosaic,
  PICTURE_BOX,
  placeholderColour,
  rememberImageSize,
} from "./imageBox.ts";
import { lightboxImage } from "./lightboxImage.ts";
import { reloadMedia } from "./mediaReload.ts";
import { OpenImage } from "./openImage.ts";
import { isPlainClick } from "./plainClick.ts";

/** Where my own message's alt texts are changed (#575). */
export type AltTextTarget = { messageId: string; channelId: string };

export function AttachmentList({
  attachments,
  editAlt,
}: {
  attachments: Attachment[];
  /** Set on my own sent message: its pictures' alt texts can be edited. */
  editAlt?: AltTextTarget;
}) {
  const [editing, setEditing] = useState<Attachment | null>(null);
  if (attachments.length === 0) return null;
  const images = attachments.filter((a) => isImageType(a.content_type));
  const others = attachments.filter((a) => !isImageType(a.content_type));
  return (
    <div className="lr-attachments">
      {images.length > 0 ? (
        <Pictures
          pictures={images}
          onEditAlt={editAlt ? setEditing : undefined}
        />
      ) : null}
      {editAlt && editing ? (
        <AltTextEditor
          key={editing.id}
          target={editAlt}
          attachment={editing}
          onClose={() => setEditing(null)}
        />
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

/** One picture in its own shape, or several as a mosaic by count (#574):
 *  1 / 2 halves / 1 big and 2 / 2×2 / 2 and 3 / 3×2, the last tile "+N"
 *  past six. Every box is known before a byte loads (imageBox.ts). */
function Pictures({
  pictures,
  onEditAlt,
}: {
  pictures: Attachment[];
  onEditAlt?: (attachment: Attachment) => void;
}) {
  const t = useT(chat);
  if (pictures.length === 1)
    return (
      <AttachmentImage
        attachment={pictures[0]!}
        gallery={pictures}
        onEditAlt={onEditAlt}
      />
    );
  const layout = mosaic(pictures.length);
  const shown = pictures.slice(0, layout.tiles);
  const hidden = pictures.length - shown.length;
  return (
    <div className={`lr-attachment-grid ${layout.name}`}>
      {shown.map((attachment, index) => (
        <AttachmentImage
          key={attachment.id}
          attachment={attachment}
          gallery={pictures}
          tile
          onEditAlt={onEditAlt}
          more={
            index === shown.length - 1 && hidden > 0
              ? t("attachment.more", { count: String(hidden) })
              : undefined
          }
          moreCount={index === shown.length - 1 ? hidden : 0}
        />
      ))}
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
  const t = useT(chat);
  const { started, link } = useDownload();
  return (
    <a href={href} download={filename} {...link}>
      {started ? (
        <span role="status">{t("attachment.downloadStarted")}</span>
      ) : (
        t("attachment.download")
      )}
    </a>
  );
}

function UploadState({ attachment }: { attachment: Attachment }) {
  const t = useT(chat);
  if (!attachment.upload || attachment.upload === "done") return null;
  if (attachment.upload === "uploading" && attachment.progress !== undefined) {
    const percent = Math.round(attachment.progress * 100);
    return (
      <span className="lr-attachment-upload has-progress" role="status">
        {t("attachment.uploadProgress", { percent: String(percent) })}
        <span
          className="lr-attachment-progress"
          aria-hidden="true"
          style={{ width: `${percent}%` }}
        />
      </span>
    );
  }
  return (
    <span className="lr-attachment-upload" role="status">
      {t(
        attachment.upload === "uploading"
          ? "attachment.uploading"
          : "attachment.waiting",
      )}
    </span>
  );
}

/** Animated pictures up to this size play inline; larger ones show their
 *  first frame and play in the viewer (#574). */
const ANIMATED_INLINE_MAX = 5 * 1024 * 1024;

/** One picture: alone in its own box, or a `tile` of a mosaic. */
function AttachmentImage({
  attachment,
  gallery,
  tile = false,
  more,
  moreCount = 0,
  onEditAlt,
}: {
  attachment: Attachment;
  gallery: Attachment[];
  tile?: boolean;
  /** "+N" over the last tile, for the pictures the mosaic leaves out. */
  more?: string;
  moreCount?: number;
  onEditAlt?: (attachment: Attachment) => void;
}) {
  const t = useT(chat);
  const openImage = useContext(OpenImage);
  const [failed, setFailed] = useState(false);
  // A new key mounts a new <img>, which asks the API again. The first
  // failure retries by itself; the second shows the error card.
  const [attempt, setAttempt] = useState(0);
  const [natural, setNatural] = useState(() => knownImageSize(attachment));
  const size = knownImageSize(attachment) ?? natural;
  const fit = !tile && size ? imageFit(size) : undefined;
  // Size known: the picture's own box, reserved before it arrives, so
  // loading does not move the list.
  const sized = fit
    ? {
        width: `min(100%, ${fit.width}px)`,
        aspectRatio: `${fit.width} / ${fit.height}`,
      }
    : undefined;
  const colour = placeholderColour(attachment);
  const original = attachment.preview_url ?? attachmentUrl(attachment.id);
  const playsInline =
    attachment.animated === true && attachment.size <= ANIMATED_INLINE_MAX;
  const thumbed =
    attachment.thumbnail === true && !attachment.preview_url && !playsInline;
  const src = thumbed ? thumbnailUrl(attachment.id, 400) : original;
  // The thumbnails are measured on the long edge, so each one's width
  // is the picture's share of it; and boxes are drawn with cover, so the
  // width the picture is drawn at is the box's, or more for a picture
  // wider than its box (cropped, or a tile, taken as square).
  const ratio = size ? size.width / size.height : 1;
  const thumbWidth = (edge: number) =>
    Math.max(1, Math.round(edge * Math.min(1, ratio)));
  const srcSet = thumbed
    ? `${thumbnailUrl(attachment.id, 400)} ${thumbWidth(400)}w, ${thumbnailUrl(attachment.id, 800)} ${thumbWidth(800)}w`
    : undefined;
  const wide = Math.max(1, ratio);
  const sizes = thumbed
    ? tile
      ? `(max-width: 600px) ${Math.ceil(50 * wide)}vw, ${Math.ceil(200 * wide)}px`
      : `${Math.ceil(fit ? Math.max(fit.width, fit.height * ratio) : PICTURE_BOX)}px`
    : undefined;
  if (failed)
    return (
      <AttachmentImageFailed
        attachment={attachment}
        style={sized}
        onRetry={() => {
          setFailed(false);
          setAttempt((n) => n + 1);
        }}
      />
    );
  const className = [
    "lr-attachment-image",
    sized ? "is-sized" : "",
    tile ? "is-tile" : "",
    !size && !tile ? "is-unsized" : "",
  ]
    .filter(Boolean)
    .join(" ");
  return (
    <div className={tile ? "lr-picture is-tile" : "lr-picture"}>
      <a
        href={original}
        // The link stays for "open in new tab"; a click shows the
        // picture here, also where no new tab exists (desktop app,
        // installed web app).
        onClick={(event) => {
          if (!isPlainClick(event)) return;
          event.preventDefault();
          if (isPendingId(attachment.id)) return;
          openImage(
            lightboxImage(attachment),
            gallery
              .filter((picture) => !isPendingId(picture.id))
              .map(lightboxImage),
          );
        }}
        // The "+N" over the last tile is part of the link's name, as a
        // label on the span inside is not read.
        aria-label={[
          t("attachment.enlargeLabel", {
            name: attachment.description ?? attachment.filename,
          }),
          more && moreCount > 0 ? more : "",
        ]
          .filter(Boolean)
          .join(", ")}
        className={className}
        style={{
          ...sized,
          ...(colour ? { backgroundColor: colour } : {}),
        }}
      >
        <img
          key={attempt}
          src={src}
          srcSet={srcSet}
          sizes={sizes}
          alt={attachment.description ?? attachment.filename}
          decoding="async"
          loading="lazy"
          onLoad={(event) => {
            if (knownImageSize(attachment)) return;
            const { naturalWidth: width, naturalHeight: height } =
              event.currentTarget;
            if (width <= 0 || height <= 0) return;
            rememberImageSize(attachment, { width, height });
            setNatural((known) =>
              known?.width === width && known.height === height
                ? known
                : { width, height },
            );
          }}
          // A picture that cannot load (storage unreachable, link expired,
          // file gone) says so instead of leaving an empty box (#348), after
          // one more try of its own.
          onError={() => {
            if (attempt === 0) setAttempt(1);
            else setFailed(true);
          }}
        />
        {attachment.animated && !playsInline ? (
          <span className="lr-attachment-badge">
            {t("attachment.animated")}
          </span>
        ) : null}
        {more && moreCount > 0 ? (
          <span className="lr-attachment-more" aria-hidden="true">
            +{moreCount}
          </span>
        ) : null}
        <UploadState attachment={attachment} />
      </a>
      <AltTextBadge
        attachment={attachment}
        onEdit={
          onEditAlt && !isPendingId(attachment.id)
            ? () => onEditAlt(attachment)
            : undefined
        }
      />
    </div>
  );
}

/** "ALT" on a picture with an alt text: a hover with the mouse or a tap
 *  shows it (#575). On my own pictures it also offers to add or change
 *  it. The text itself reaches screen readers as the picture's `alt`. */
function AltTextBadge({
  attachment,
  onEdit,
}: {
  attachment: Attachment;
  onEdit?: () => void;
}) {
  const t = useT(chat);
  const [open, setOpen] = useState(false);
  const hovered = useRef(false);
  const badge = useRef<HTMLButtonElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);
  // A mouse leaving for the bubble crosses a small gap: closing waits.
  const closing = useRef<number | undefined>(undefined);
  // Where the bubble goes: fixed to the badge, outside the mosaic's clip.
  const [at, setAt] = useState<{
    top?: number;
    bottom?: number;
    left: number;
    maxHeight: number;
  }>();
  const text = attachment.description;
  const show = (next: boolean) => {
    const box = badge.current?.getBoundingClientRect();
    if (next && box) {
      // Below the badge, or above it where more room is (a picture low
      // in the window); never past the window's edge.
      const below = window.innerHeight - box.bottom - 12;
      const above = box.top - 12;
      const left = Math.max(8, Math.min(box.left, window.innerWidth - 328));
      const cap = Math.min(320, window.innerHeight * 0.4);
      setAt(
        below >= 160 || below >= above
          ? { top: box.bottom + 4, left, maxHeight: Math.min(below, cap) }
          : {
              bottom: window.innerHeight - box.top + 4,
              left,
              maxHeight: Math.min(above, cap),
            },
      );
    }
    setOpen(next);
  };
  useEffect(() => () => window.clearTimeout(closing.current), []);
  // The list scrolls under a fixed bubble: it goes. Scrolling the bubble
  // itself (a long text) keeps it.
  useEffect(() => {
    if (!open) return;
    const hide = (event: Event) => {
      if (
        event.target instanceof Node &&
        bubble.current?.contains(event.target)
      )
        return;
      setOpen(false);
    };
    window.addEventListener("scroll", hide, { capture: true, passive: true });
    window.addEventListener("resize", hide);
    return () => {
      window.removeEventListener("scroll", hide, { capture: true });
      window.removeEventListener("resize", hide);
    };
  }, [open]);
  if (!text && !onEdit) return null;
  if (!text)
    return (
      <button
        type="button"
        className="lr-alt-badge is-empty"
        onClick={onEdit}
        aria-label={`${t("altText.add")}: ${attachment.filename}`}
        title={t("altText.add")}
      >
        {t("altText.badge")}
      </button>
    );
  return (
    <span
      className="lr-alt"
      onPointerEnter={() => window.clearTimeout(closing.current)}
      onPointerLeave={(event) => {
        if (event.pointerType !== "mouse") return;
        window.clearTimeout(closing.current);
        closing.current = window.setTimeout(() => {
          hovered.current = false;
          show(false);
        }, 200);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.stopPropagation();
          show(false);
        }
      }}
    >
      {/* No expanded state for screen readers: the text it shows is
          already the picture's name, and is hidden from them here. */}
      <button
        ref={badge}
        type="button"
        className="lr-alt-badge"
        aria-label={t("altText.show")}
        onPointerEnter={(event) => {
          if (event.pointerType !== "mouse") return;
          hovered.current = true;
          show(true);
        }}
        // A mouse opened it on hover already: its click keeps it open.
        onClick={() => show(hovered.current ? true : !open)}
      >
        {t("altText.badge")}
      </button>
      {open ? (
        <span ref={bubble} className="lr-alt-text" style={at}>
          {/* First, so a long text does not push it out of reach. */}
          {onEdit ? (
            <button type="button" onClick={onEdit}>
              {t("altText.edit")}
            </button>
          ) : null}
          {/* Already read out as the picture's alt. */}
          <span aria-hidden="true">{text}</span>
        </span>
      ) : null}
    </span>
  );
}

/** The alt text dialog for one of my sent pictures; saved as an edit. */
function AltTextEditor({
  target,
  attachment,
  onClose,
}: {
  target: AltTextTarget;
  attachment: Attachment;
  onClose: () => void;
}) {
  const edit = useEditAltText(target.channelId);
  return (
    <AltTextDialog
      open
      name={attachment.filename}
      preview={
        attachment.thumbnail
          ? thumbnailUrl(attachment.id, 400)
          : attachmentUrl(attachment.id)
      }
      initial={attachment.description ?? ""}
      pending={edit.isPending}
      onClose={onClose}
      onSave={(text) => {
        const scope = takeStamp();
        if (!scope) return;
        edit.mutate(
          {
            messageId: target.messageId,
            attachmentId: attachment.id,
            text,
            scope,
          },
          {
            onSuccess: onClose,
            onError: (error) =>
              notifyError(error, translate(chat, "altText.saveFailed")),
          },
        );
      }}
    />
  );
}

/** The box of a picture that did not load: what failed, a retry, and the
 *  file itself, which may still arrive as a download. Same height as a
 *  picture still loading, so the list does not move. */
export function AttachmentImageFailed({
  attachment,
  onRetry,
  style,
}: {
  attachment: Attachment;
  onRetry: () => void;
  /** The box of the picture it replaces. */
  style?: CSSProperties;
}) {
  const t = useT(chat);
  return (
    <div className="lr-attachment-image is-failed" role="group" style={style}>
      <p className="lr-attachment-image-error">
        <PaperclipIcon size={15} />
        <span className="lr-attachment-image-name">{attachment.filename}</span>
        <span role="status">{t("attachment.imageFailed")}</span>
        <span className="lr-attachment-image-actions">
          <button type="button" onClick={onRetry}>
            {t("attachment.reload")}
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
  const t = useT(chat);
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
          <span role="status">{t("attachment.downloadStarted")}</span>
        ) : (
          <span>{formatSize(attachment.size)}</span>
        )}
      </figcaption>
    </figure>
  );
}

function AttachmentText({ attachment }: { attachment: Attachment }) {
  const t = useT(chat);
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
      onToggle={(event) => {
        const opened = event.currentTarget.open;
        setOpen(opened);
        // Opening again fetches again: a failure before is not this answer.
        if (opened) setError(false);
      }}
    >
      <summary>
        <PaperclipIcon size={15} />
        <span>{attachment.filename}</span>
        <span>{formatSize(attachment.size)}</span>
        <UploadState attachment={attachment} />
      </summary>
      {error ? (
        <p className="lr-attachment-text-state">
          {t("attachment.previewFailed")}{" "}
          <DownloadLink
            href={attachmentDownloadUrl(attachment.id)}
            filename={attachment.filename}
          />
        </p>
      ) : text === null ? (
        <p className="lr-attachment-text-state">
          {t("attachment.previewLoading")}
        </p>
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
  const t = useT(chat);
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
        <span role="status">{t("attachment.downloadStarted")}</span>
      ) : (
        <span>{formatSize(attachment.size)}</span>
      )}
      <UploadState attachment={attachment} />
    </a>
  );
}
