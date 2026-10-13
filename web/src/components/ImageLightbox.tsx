import { useEffect, useRef } from "react";

import { CloseIcon } from "./Icons.tsx";

export type LightboxImage = {
  id: string;
  src: string;
  filename: string;
  /** Saves the file (`?download=1`), see `attachmentDownloadUrl`. */
  download: string;
};

/**
 * A chat image at full size, inside the app: a new tab does not exist in the
 * desktop app or the installed web app. Esc, "Schließen" or a click next to
 * the image closes it; focus starts on "Schließen" and returns to the opener.
 */
export function ImageLightbox({
  image,
  onClose,
}: {
  image: LightboxImage;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const close = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    if (!dialog.open) dialog.showModal();
    // Not the download link, which showModal would pick as the first one.
    close.current?.focus({ preventScroll: true });
    return () => {
      if (dialog.open) dialog.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={ref}
      aria-label={image.filename}
      className="lr-lightbox"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        // The backdrop and the empty space around the picture.
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <img src={image.src} alt={image.filename} />
      <div className="lr-lightbox-bar">
        <span title={image.filename}>{image.filename}</span>
        {/* The API answers this link with `Content-Disposition: attachment`
            (from MinIO after its redirect too), so the file is saved and the
            app, with a running call, stays where it is. */}
        <a href={image.download} download={image.filename}>
          Herunterladen
        </a>
        <button
          ref={close}
          type="button"
          aria-label="Schließen"
          onClick={onClose}
        >
          <CloseIcon size={18} />
        </button>
      </div>
    </dialog>
  );
}
