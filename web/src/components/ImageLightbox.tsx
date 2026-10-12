import { useEffect, useRef } from "react";

import { ChevronIcon, CloseIcon } from "./Icons.tsx";

export type LightboxImage = { id: string; src: string; filename: string };

/**
 * A chat image at full size, inside the app: a new tab does not exist in the
 * desktop app or the installed web app. Esc or a click next to the image
 * closes it, the arrow keys step through the message's images.
 */
export function ImageLightbox({
  images,
  index,
  downloadUrl,
  onIndex,
  onClose,
}: {
  images: LightboxImage[];
  index: number;
  downloadUrl: (id: string) => string;
  onIndex: (index: number) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const image = images[index];
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    if (!dialog.open) dialog.showModal();
    return () => {
      if (dialog.open) dialog.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);
  if (!image) return null;
  const many = images.length > 1;
  const step = (delta: number) =>
    onIndex((index + delta + images.length) % images.length);
  return (
    <dialog
      ref={ref}
      aria-label={image.filename}
      className="lr-lightbox"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onKeyDown={(event) => {
        if (!many) return;
        if (event.key === "ArrowLeft") step(-1);
        else if (event.key === "ArrowRight") step(1);
      }}
      onClick={(event) => {
        // The backdrop and the empty space around the picture.
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <img src={image.src} alt={image.filename} />
      <div className="lr-lightbox-bar">
        <span title={image.filename}>{image.filename}</span>
        {many ? (
          <span className="lr-lightbox-count">
            {index + 1} / {images.length}
          </span>
        ) : null}
        <a href={downloadUrl(image.id)} download={image.filename}>
          Herunterladen
        </a>
        <button type="button" aria-label="Schließen" onClick={onClose}>
          <CloseIcon size={18} />
        </button>
      </div>
      {many ? (
        <>
          <button
            type="button"
            className="lr-lightbox-step is-previous"
            aria-label="Vorheriges Bild"
            onClick={() => step(-1)}
          >
            <ChevronIcon size={22} />
          </button>
          <button
            type="button"
            className="lr-lightbox-step is-next"
            aria-label="Nächstes Bild"
            onClick={() => step(1)}
          >
            <ChevronIcon size={22} />
          </button>
        </>
      ) : null}
    </dialog>
  );
}
