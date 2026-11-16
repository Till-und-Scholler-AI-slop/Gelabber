import {
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";

import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { common } from "../i18n/catalogs/common.ts";
import { formatSize } from "../messages/rules.ts";
import { isTauriApp } from "../tauriApp.ts";
import { DownloadLink } from "./Attachments.tsx";
import { CloseIcon } from "./Icons.tsx";

export type LightboxImage = {
  id: string;
  /** Shown first: the 800 px thumbnail where there is one (#574). */
  src: string;
  /** The original, swapped in once it has loaded; absent when `src` is it. */
  full?: string;
  filename: string;
  /** The uploader's alt text (#575), shown under the picture. */
  alt?: string;
  /** Bytes of the original, for the bar. */
  size?: number;
  /** Saves the file (`?download=1`), see `attachmentDownloadUrl`. Absent
   *  for a link preview's picture, which the API serves only for display. */
  download?: string;
  /** "Open original" in a new tab, where tabs exist. */
  open?: string;
  /** The original's size as shown, from the API: the box the thumbnail
   *  and the original fill alike, and what "actual size" means. */
  width?: number;
  height?: number;
};

/** The most a picture is enlarged, relative to fitting the screen. */
const ZOOM_MAX = 8;
/** Pixels a finger moves before a touch counts as a swipe, not a tap. */
const SWIPE_MIN = 60;
const DOUBLE_TAP_MS = 300;

type View = { scale: number; x: number; y: number };
const FIT: View = { scale: 1, x: 0, y: 0 };

/** Tabs exist outside the installed web app and the desktop app. The
 *  old Tauri shell is no standalone window, but its system browser has no
 *  session cookie. */
function hasTabs(): boolean {
  if (isTauriApp()) return false;
  try {
    return !window.matchMedia("(display-mode: standalone)").matches;
  } catch {
    return true;
  }
}

/**
 * Chat pictures at full size, inside the app: a new tab does not exist in
 * the desktop app or the installed web app. `gallery` holds every picture
 * of the message; ← / →, the arrow buttons or a swipe step through them.
 * Wheel, pinch, double tap or a click zoom in, a drag pans. Esc,
 * "Schließen", a click next to the picture, a swipe down or the back
 * button (a history entry while open) close it; focus starts on
 * "Schließen" and returns to the opener, or, when that is gone (its row
 * scrolled away or was deleted), to `fallbackFocus`.
 */
export function ImageLightbox({
  image,
  gallery,
  onClose,
  fallbackFocus,
}: {
  image: LightboxImage;
  gallery?: LightboxImage[];
  onClose: () => void;
  fallbackFocus?: () => void;
}) {
  const t = useT(common);
  const tc = useT(chat);
  const images = gallery && gallery.length > 0 ? gallery : [image];
  const [index, setIndex] = useState(() =>
    Math.max(
      0,
      images.findIndex((candidate) => candidate.id === image.id),
    ),
  );
  const current = images[Math.min(index, images.length - 1)]!;
  const [view, setView] = useState<View>(FIT);
  // The original once it has loaded, per picture.
  const [loaded, setLoaded] = useState<Record<string, true>>({});
  const [drag, setDrag] = useState({ x: 0, y: 0 });
  // A finger or the mouse is down: the picture follows it, no easing.
  const [gesturing, setGesturing] = useState(false);
  // The stage's size, for the box a picture of known size fills.
  const [stageSize, setStageSize] = useState<{ w: number; h: number }>();

  const fallback = useRef(fallbackFocus);
  const close = useRef(onClose);
  useEffect(() => {
    fallback.current = fallbackFocus;
    close.current = onClose;
  });
  const ref = useRef<HTMLDialogElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const picture = useRef<HTMLImageElement>(null);
  // Counts mounts, so a cleanup can tell a remount (StrictMode) from the
  // end and leave the history entry to the next mount.
  const mounts = useRef(0);

  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    // The content box: client sizes include the stage's padding.
    const measure = () => {
      const style = getComputedStyle(element);
      const px = (value: string) => parseFloat(value) || 0;
      setStageSize({
        w: element.clientWidth - px(style.paddingLeft) - px(style.paddingRight),
        h:
          element.clientHeight - px(style.paddingTop) - px(style.paddingBottom),
      });
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    if (!dialog.open) dialog.showModal();
    // Not the download link, which showModal would pick as the first one.
    closeButton.current?.focus({ preventScroll: true });
    // The back button (Android, browser) closes the picture, not the page:
    // an entry of our own, with the router's state, so it sees no move.
    // One entry, also when mounted twice.
    const mount = ++mounts.current;
    let popped = false;
    const onPop = () => {
      popped = true;
      close.current();
    };
    if (!window.history.state?.lightbox)
      window.history.pushState({ ...window.history.state, lightbox: true }, "");
    window.addEventListener("popstate", onPop);
    return () => {
      window.removeEventListener("popstate", onPop);
      // Later, and only if no new mount took the entry over meanwhile.
      if (!popped)
        window.setTimeout(() => {
          if (mounts.current === mount && window.history.state?.lightbox)
            window.history.back();
        }, 0);
      if (dialog.open) dialog.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
      else fallback.current?.();
    };
  }, []);

  // The original, fetched next to the thumbnail on screen.
  useEffect(() => {
    if (!current.full || loaded[current.id]) return;
    const original = new Image();
    let live = true;
    original.onload = () => {
      if (live) setLoaded((done) => ({ ...done, [current.id]: true }));
    };
    original.src = current.full;
    return () => {
      live = false;
      // Stepped on: the original of a picture passed by is not wanted.
      original.onload = null;
      original.src = "";
    };
  }, [current.id, current.full, loaded]);

  const go = (step: number) => {
    if (images.length < 2) return;
    setIndex((at) => (at + step + images.length) % images.length);
    setView(FIT);
    setDrag({ x: 0, y: 0 });
  };

  /** Zoom to `scale` with the point `at` (client coordinates) kept where
   *  it is on screen. */
  const zoomTo = (scale: number, at?: { x: number; y: number }) => {
    const next = Math.min(ZOOM_MAX, Math.max(1, scale));
    setView((was) => {
      if (next === 1) return FIT;
      const box = stage.current?.getBoundingClientRect();
      if (!at || !box) return { ...was, scale: next };
      const px = at.x - (box.left + box.width / 2);
      const py = at.y - (box.top + box.height / 2);
      const ratio = next / was.scale;
      return {
        scale: next,
        x: px - (px - was.x) * ratio,
        y: py - (py - was.y) * ratio,
      };
    });
  };

  /** The scale at which the picture shows its own pixels, at least 2. */
  const actualSize = () => {
    const img = picture.current;
    if (!img || !img.clientWidth) return 2;
    // The original's width, also while the thumbnail is on screen.
    const natural = current.width ?? img.naturalWidth;
    return Math.max(2, natural / img.clientWidth);
  };

  // Pointers on the stage: one drags (pan when zoomed, else swipe), two
  // pinch.
  const pointers = useRef(new Map<number, { x: number; y: number }>());
  const gesture = useRef<{
    start: { x: number; y: number };
    view: View;
    pinch?: { distance: number; scale: number };
    moved: boolean;
    touch: boolean;
    /** Went down on the picture. Pointer capture makes the stage the
     *  target of every later event, so this is decided here. */
    onPicture: boolean;
    /** The finger left over from a pinch: it pans when zoomed, but never
     *  drags, steps or closes at fit. */
    noSwipe?: boolean;
  } | null>(null);
  const lastTap = useRef(0);

  const distance = () => {
    const [a, b] = [...pointers.current.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  };

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    if ((event.target as HTMLElement).closest("button, a")) return;
    pointers.current.set(event.pointerId, {
      x: event.clientX,
      y: event.clientY,
    });
    const onPicture = event.target === picture.current;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setGesturing(true);
    if (pointers.current.size === 2) {
      gesture.current = {
        start: { x: event.clientX, y: event.clientY },
        view,
        pinch: { distance: distance(), scale: view.scale },
        moved: true,
        touch: true,
        onPicture,
      };
    } else {
      gesture.current = {
        start: { x: event.clientX, y: event.clientY },
        view,
        moved: false,
        touch: event.pointerType !== "mouse",
        onPicture,
      };
    }
  };

  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointers.current.has(event.pointerId) || !gesture.current) return;
    pointers.current.set(event.pointerId, {
      x: event.clientX,
      y: event.clientY,
    });
    const g = gesture.current;
    if (g.pinch && pointers.current.size === 2) {
      const [a, b] = [...pointers.current.values()];
      zoomTo((g.pinch.scale * distance()) / Math.max(g.pinch.distance, 1), {
        x: (a!.x + b!.x) / 2,
        y: (a!.y + b!.y) / 2,
      });
      return;
    }
    const dx = event.clientX - g.start.x;
    const dy = event.clientY - g.start.y;
    if (Math.hypot(dx, dy) > 6) g.moved = true;
    if (!g.moved) return;
    if (g.view.scale > 1) {
      setView({ ...g.view, x: g.view.x + dx, y: g.view.y + dy });
    } else if (g.touch && !g.noSwipe) {
      // The picture follows the finger: sideways to step, down to close.
      setDrag({ x: dx, y: Math.max(0, dy) });
    }
  };

  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (!pointers.current.delete(event.pointerId)) return;
    const g = gesture.current;
    if (!g) return;
    if (g.pinch) {
      const [rest] = pointers.current.values();
      if (!rest) {
        gesture.current = null;
        setGesturing(false);
        return;
      }
      // The finger still down pans on from where it is, at the zoom the
      // pinch left; it starts no swipe.
      gesture.current = {
        start: rest,
        view,
        moved: true,
        touch: true,
        onPicture: false,
        noSwipe: true,
      };
      setDrag({ x: 0, y: 0 });
      return;
    }
    gesture.current = null;
    setGesturing(false);
    const dx = event.clientX - g.start.x;
    const dy = event.clientY - g.start.y;
    setDrag({ x: 0, y: 0 });
    if (g.moved) {
      if (g.view.scale > 1 || !g.touch || g.noSwipe) return;
      if (Math.abs(dx) > SWIPE_MIN && Math.abs(dx) > Math.abs(dy))
        go(dx < 0 ? 1 : -1);
      else if (dy > SWIPE_MIN * 1.5 && dy > Math.abs(dx)) onClose();
      return;
    }
    // A tap or click.
    const { onPicture } = g;
    if (g.touch) {
      const now = event.timeStamp;
      if (now - lastTap.current < DOUBLE_TAP_MS) {
        lastTap.current = 0;
        zoomTo(view.scale > 1 ? 1 : 2.5, {
          x: event.clientX,
          y: event.clientY,
        });
        return;
      }
      lastTap.current = now;
      if (!onPicture && view.scale === 1) onClose();
      return;
    }
    if (!onPicture) {
      if (view.scale === 1) onClose();
      return;
    }
    // Click: the picture's own pixels, and back.
    zoomTo(view.scale > 1 ? 1 : actualSize(), {
      x: event.clientX,
      y: event.clientY,
    });
  };

  // Wheel zoom: a listener of our own, as React's is passive.
  useEffect(() => {
    const element = stage.current;
    if (!element) return;
    const onWheel = (event: WheelEvent) => {
      event.preventDefault();
      setView((was) => {
        const next = Math.min(
          ZOOM_MAX,
          Math.max(1, was.scale * Math.exp(-event.deltaY * 0.002)),
        );
        if (next === 1) return FIT;
        const box = element.getBoundingClientRect();
        const px = event.clientX - (box.left + box.width / 2);
        const py = event.clientY - (box.top + box.height / 2);
        const ratio = next / was.scale;
        return {
          scale: next,
          x: px - (px - was.x) * ratio,
          y: py - (py - was.y) * ratio,
        };
      });
    };
    element.addEventListener("wheel", onWheel, { passive: false });
    return () => element.removeEventListener("wheel", onWheel);
  }, []);

  const many = images.length > 1;
  const src = loaded[current.id] && current.full ? current.full : current.src;
  const transform =
    view.scale > 1
      ? `translate(${view.x}px, ${view.y}px) scale(${view.scale})`
      : drag.x || drag.y
        ? `translate(${drag.x}px, ${drag.y}px)`
        : undefined;
  // A picture of known size fills its fitted box from the first (800 px)
  // thumbnail on, so the original takes the same place.
  const fitted =
    current.width && current.height && stageSize?.w && stageSize.h
      ? Math.min(1, stageSize.w / current.width, stageSize.h / current.height)
      : undefined;
  const box = fitted
    ? { width: current.width! * fitted, height: current.height! * fitted }
    : undefined;
  const fading = drag.y > 0 ? Math.max(0.3, 1 - drag.y / 400) : undefined;
  const label = many
    ? tc("lightbox.label", {
        n: String(index + 1),
        total: String(images.length),
        name: current.filename,
      })
    : current.filename;
  return (
    <dialog
      ref={ref}
      aria-label={label}
      className="lr-lightbox"
      style={fading ? { opacity: fading } : undefined}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onKeyDown={(event) => {
        if (event.key === "ArrowRight") {
          event.preventDefault();
          go(1);
        } else if (event.key === "ArrowLeft") {
          event.preventDefault();
          go(-1);
        }
      }}
    >
      <div
        ref={stage}
        className={
          view.scale > 1 ? "lr-lightbox-stage is-zoomed" : "lr-lightbox-stage"
        }
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={(event) => {
          pointers.current.delete(event.pointerId);
          gesture.current = null;
          setGesturing(false);
          setDrag({ x: 0, y: 0 });
        }}
      >
        <img
          ref={picture}
          key={current.id}
          src={src}
          alt={current.alt ?? current.filename}
          draggable={false}
          className={drag.x || drag.y || gesturing ? "is-dragging" : undefined}
          style={box || transform ? { ...box, transform } : undefined}
        />
      </div>
      {current.alt ? (
        // Read out as the picture's alt already; shown for everyone else.
        <p className="lr-lightbox-alt" aria-hidden="true">
          {current.alt}
        </p>
      ) : null}
      {many ? (
        <>
          <button
            type="button"
            className="lr-lightbox-step is-previous"
            aria-label={tc("lightbox.previous")}
            onClick={() => go(-1)}
          >
            <span aria-hidden="true">‹</span>
          </button>
          <button
            type="button"
            className="lr-lightbox-step is-next"
            aria-label={tc("lightbox.next")}
            onClick={() => go(1)}
          >
            <span aria-hidden="true">›</span>
          </button>
        </>
      ) : null}
      <div className="lr-lightbox-bar">
        <span title={current.filename}>
          {current.filename}
          {current.size ? (
            <span className="lr-lightbox-size">
              {" "}
              {formatSize(current.size)}
            </span>
          ) : null}
        </span>
        {many ? (
          <span className="lr-lightbox-count" aria-hidden="true">
            {tc("lightbox.count", {
              n: String(index + 1),
              total: String(images.length),
            })}
          </span>
        ) : null}
        {current.open && hasTabs() ? (
          <a href={current.open} target="_blank" rel="noreferrer">
            {tc("lightbox.openOriginal")}
          </a>
        ) : null}
        {/* The API answers this link with `Content-Disposition: attachment`
            (from MinIO after its redirect too), so the file is saved and the
            app, with a running call, stays where it is. */}
        {current.download ? (
          <DownloadLink
            key={current.id}
            href={current.download}
            filename={current.filename}
          />
        ) : null}
        <button
          ref={closeButton}
          type="button"
          aria-label={t("close")}
          onClick={onClose}
        >
          <CloseIcon size={18} />
        </button>
      </div>
    </dialog>
  );
}
