// Local or remote video tile. srcObject is set in an effect so the
// preview can appear in the same frame as the stream, without SDP.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import { CollapseIcon, ExpandIcon } from "../components/Icons.tsx";
import {
  hasNativeFeature,
  subscribeNativeFeatures,
} from "./native/features.ts";
import { NativeVideoCanvas } from "./native/NativeVideoCanvas.tsx";
import { isNativeStream, type NativeTrack } from "./native/tracks.ts";
import {
  nativeDisplayTrack,
  nativeVideoLimit,
  nativeVideoSuspended,
  subscribeNativeVideoLimit,
  subscribeNativeVideoSuspensions,
} from "./native/videoFeed.ts";
import {
  closeNativeViewer,
  nativeVideoTrack,
  nativeViewerOpen,
  nativeViewersVersion,
  openNativeViewer,
  subscribeNativeViewers,
} from "./native/viewer.ts";
import "./viewer.css";

type VoiceTileProps = {
  stream: MediaStream | null;
  label: string;
  mirror?: boolean;
  screen?: boolean;
  live?: boolean;
  expanded?: boolean;
  onToggleExpand?: () => void;
  sourceWatch?: { watching: boolean; toggle: () => void };
  sourceAudioNotice?: string;
};

export function VoiceTile(props: VoiceTileProps) {
  const [viewing, setViewing] = useState(false);
  return (
    <>
      <VideoSurface {...props} onEnlarge={() => setViewing(true)} />
      {viewing && <StreamViewer {...props} onClose={() => setViewing(false)} />}
    </>
  );
}

function StreamViewer(props: VoiceTileProps & { onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    const opener =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (opener?.isConnected) opener.focus({ preventScroll: true });
    };
  }, []);
  return (
    <dialog
      ref={dialogRef}
      className="stream-viewer"
      aria-label={`Große Ansicht: ${props.label}`}
      onCancel={(event) => {
        event.preventDefault();
        props.onClose();
      }}
    >
      <header className="stream-viewer-header">
        <strong>{props.label}</strong>
        <button
          type="button"
          onClick={props.onClose}
          autoFocus
          data-viewer-close=""
        >
          Schließen <span aria-hidden="true">×</span>
        </button>
      </header>
      <VideoSurface {...props} expanded viewer onToggleExpand={undefined} />
    </dialog>
  );
}

function VideoSurface({
  stream: given,
  label,
  mirror,
  screen,
  live,
  expanded,
  onToggleExpand,
  onEnlarge,
  viewer,
  sourceWatch,
  sourceAudioNotice,
}: VoiceTileProps & { onEnlarge?: () => void; viewer?: boolean }) {
  // Native streams (desktop app) are decoded by the native core and cannot
  // feed a <video>. An app that hands frames to the page has them drawn on
  // a canvas in its place; an app up to 0.5.x keeps the placeholder.
  const nativeVideo = isNativeStream(given);
  const stream = nativeVideo ? null : given;
  const inPage = useSyncExternalStore(
    subscribeNativeFeatures,
    videoFrames,
    videoFrames,
  );
  const track = nativeVideo && inPage ? nativeDisplayTrack(given) : null;
  // A stream that plays in its viewer window is not drawn here as well: the
  // page has no frames of it meanwhile (videoFeed.ts).
  const consumer = track?.handle.consumer;
  const suspended = () =>
    consumer !== undefined && nativeVideoSuspended(consumer);
  const inWindow = useSyncExternalStore(
    subscribeNativeVideoSuspensions,
    suspended,
    suspended,
  );
  const drawn = inWindow ? null : track;
  const [painted, setPainted] = useState<NativeTrack | null>(null);
  const showing = stream !== null || (drawn !== null && painted === drawn);
  const limit = useSyncExternalStore(
    subscribeNativeVideoLimit,
    nativeVideoLimit,
    nativeVideoLimit,
  );
  const [windowFailed, setWindowFailed] = useState<NativeTrack | null>(null);
  const figureRef = useRef<HTMLElement>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [fullscreenError, setFullscreenError] = useState(false);
  useEffect(() => {
    const update = () =>
      setFullscreen(document.fullscreenElement === figureRef.current);
    document.addEventListener("fullscreenchange", update);
    return () => document.removeEventListener("fullscreenchange", update);
  }, []);
  const toggleFullscreen = async () => {
    setFullscreenError(false);
    try {
      if (document.fullscreenElement === figureRef.current)
        await document.exitFullscreen();
      else if (figureRef.current?.requestFullscreen)
        await figureRef.current.requestFullscreen();
      else throw new Error("Fullscreen unavailable");
    } catch {
      setFullscreenError(true);
    }
  };
  const ref = useRef<HTMLVideoElement>(null);
  const [blockedStream, setBlockedStream] = useState<MediaStream | null>(null);
  const play = useCallback(() => {
    const el = ref.current;
    if (!el || !stream) return;
    const failed = (error: unknown) => {
      if (
        el.srcObject !== stream ||
        (error as { name?: string })?.name === "AbortError"
      )
        return;
      setBlockedStream(stream);
    };
    try {
      void el
        .play()
        ?.then(() => {
          if (el.srcObject === stream) setBlockedStream(null);
        })
        .catch(failed);
    } catch (error) {
      failed(error);
    }
  }, [stream]);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.srcObject = stream;
    if (stream) {
      play();
      const onUnmute = () => play();
      stream.getVideoTracks().forEach((track) => {
        track.addEventListener("unmute", onUnmute);
      });
      return () => {
        stream.getVideoTracks().forEach((track) => {
          track.removeEventListener("unmute", onUnmute);
        });
        el.srcObject = null;
      };
    }
    return () => {
      el.srcObject = null;
    };
  }, [stream, play]);

  return (
    <figure
      ref={figureRef}
      className={[
        "voice-video-tile",
        viewer ? "voice-video-viewer" : "",
        "relative overflow-hidden rounded-lg bg-neutral-900 dark:bg-neutral-700 text-white",
        screen || expanded ? "aspect-video w-full" : "aspect-video",
        expanded && !viewer ? "min-h-[40vh] sm:min-h-[56vh]" : "",
      ].join(" ")}
    >
      {drawn ? (
        <NativeVideoCanvas
          key={drawn.id}
          track={drawn}
          onDoubleClick={() => void toggleFullscreen()}
          onPainted={(done) => setPainted(done ? drawn : null)}
          className={[
            "size-full",
            screen || expanded ? "object-contain" : "object-cover",
            mirror ? "-scale-x-100" : "",
            painted === drawn ? "" : "opacity-0",
          ].join(" ")}
        />
      ) : (
        <video
          ref={ref}
          autoPlay
          playsInline
          muted
          onDoubleClick={() => void toggleFullscreen()}
          className={[
            "size-full",
            screen || expanded ? "object-contain" : "object-cover",
            mirror ? "-scale-x-100" : "",
            stream ? "" : "opacity-0",
          ].join(" ")}
        />
      )}
      {stream && blockedStream === stream ? (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/65 p-3 text-sm">
          <p role="status">Die Videowiedergabe ist blockiert.</p>
          <button
            type="button"
            onClick={play}
            className="rounded-md bg-white px-3 py-2 font-medium text-neutral-900"
          >
            Wiedergabe starten
          </button>
        </div>
      ) : null}
      {sourceWatch && !sourceWatch.watching ? (
        <div className="voice-source-watch-overlay">
          <button type="button" onClick={sourceWatch.toggle}>
            Zuschauen
          </button>
        </div>
      ) : null}
      {sourceAudioNotice ? (
        <p
          role="status"
          className="voice-source-audio-notice voice-source-audio-tile"
        >
          {sourceAudioNotice}
        </p>
      ) : null}
      {!showing ? (
        <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 p-3 text-center text-sm text-neutral-400 dark:text-neutral-500">
          <span>{label}</span>
          {nativeVideo && !inPage ? (
            <NativeVideo stream={given} label={label} />
          ) : null}
          {inWindow ? (
            <span role="status" className="text-xs">
              Läuft im eigenen Fenster.
            </span>
          ) : null}
        </div>
      ) : null}
      <figcaption className="absolute inset-x-0 bottom-0 truncate bg-black/50 px-2 py-1 text-left text-xs">
        {label}
      </figcaption>
      {live ? (
        <span className="absolute top-2 left-2 rounded bg-red-600 px-1.5 py-0.5 text-[10px] font-semibold tracking-wide uppercase">
          Live
        </span>
      ) : null}
      {drawn && limit ? (
        <span
          role="status"
          title={LIMIT_REASON[limit]}
          className={[
            "absolute left-2 rounded bg-black/60 px-1.5 py-0.5 text-[10px]",
            live ? "top-8" : "top-2",
          ].join(" ")}
        >
          Geringe Bildqualität
        </span>
      ) : null}
      <div className="voice-video-actions">
        {sourceWatch?.watching && (
          <button type="button" onClick={sourceWatch.toggle}>
            Nicht mehr zuschauen
          </button>
        )}
        {onToggleExpand && (
          <button
            type="button"
            onClick={onToggleExpand}
            aria-pressed={expanded ?? false}
            title={expanded ? "Rasteransicht" : "Im Raum hervorheben"}
            aria-label={expanded ? "Rasteransicht" : "Im Raum hervorheben"}
          >
            {expanded ? <CollapseIcon size={16} /> : <ExpandIcon size={16} />}
          </button>
        )}
        {track && consumer !== undefined ? (
          <OwnWindow
            track={track}
            label={label}
            closeOnly={viewer ?? false}
            onFailed={(failed) => setWindowFailed(failed ? track : null)}
          />
        ) : null}
        {onEnlarge && !fullscreen && (
          <button type="button" onClick={onEnlarge}>
            Vergrößern
          </button>
        )}
        <button
          type="button"
          onClick={() => void toggleFullscreen()}
          aria-label={fullscreen ? "Vollbild verlassen" : "Vollbild"}
          title="Vollbild auch per Doppelklick"
        >
          <ExpandIcon size={16} /> {fullscreen ? "Verkleinern" : "Vollbild"}
        </button>
      </div>
      {fullscreenError && (
        <p role="status" className="voice-fullscreen-error">
          Vollbild ist hier nicht verfügbar.
          {onEnlarge ? (
            <button type="button" onClick={onEnlarge}>
              Große Ansicht öffnen
            </button>
          ) : (
            " Du kannst die große Ansicht weiter nutzen."
          )}
        </p>
      )}
      {track && windowFailed === track && !fullscreenError ? (
        <p role="status" className="voice-fullscreen-error">
          Das Videofenster konnte nicht geöffnet werden.
        </p>
      ) : null}
    </figure>
  );
}

const videoFrames = () => hasNativeFeature("video-frames");

const LIMIT_REASON = {
  transport:
    "Der Server blockiert die schnelle Bildübertragung der Desktop-App (Content-Security-Policy). Das Bild bleibt deshalb klein.",
  renderer: "WebGL ist hier nicht verfügbar. Das Bild bleibt deshalb klein.",
};

/** Someone's stream in a window of its own instead of in its tiles: for a
 * second monitor or a tiling compositor. `closeOnly`: the large view offers
 * no window, but one that is open can be closed where its stream would be. */
function OwnWindow({
  track,
  label,
  closeOnly,
  onFailed,
}: {
  track: NativeTrack;
  label: string;
  closeOnly: boolean;
  onFailed: (failed: boolean) => void;
}) {
  useSyncExternalStore(
    subscribeNativeViewers,
    nativeViewersVersion,
    nativeViewersVersion,
  );
  const consumer = track.handle.consumer;
  if (consumer === undefined) return null;
  const open = nativeViewerOpen(consumer);
  if (closeOnly && !open) return null;
  return (
    <button
      type="button"
      aria-pressed={open}
      onClick={(event) => {
        onFailed(false);
        if (open) {
          // In the large view the switch goes with the window; the focus
          // goes to the dialog's own close button, not out of the dialog.
          if (closeOnly)
            event.currentTarget
              .closest("dialog")
              ?.querySelector<HTMLElement>("[data-viewer-close]")
              ?.focus();
          closeNativeViewer(consumer);
        } else
          openNativeViewer(track, `${label} – Gelabber`).catch(() =>
            onFailed(true),
          );
      }}
    >
      {open ? "Fenster schließen" : "Eigenes Fenster"}
    </button>
  );
}

/** Desktop apps up to 0.5.x: remote video only in a native window, no
 * picture of the own camera or screen. */
function NativeVideo({
  stream,
  label,
}: {
  stream: MediaStream | null;
  label: string;
}) {
  useSyncExternalStore(
    subscribeNativeViewers,
    nativeViewersVersion,
    nativeViewersVersion,
  );
  const [failed, setFailed] = useState(false);
  const track = nativeVideoTrack(stream);
  const consumer = track?.handle.consumer;
  if (!track || consumer === undefined)
    return (
      <span role="status" className="text-xs">
        Keine Vorschau in der Desktop-App.
      </span>
    );
  const open = nativeViewerOpen(consumer);
  return (
    <>
      <button
        type="button"
        className="rounded-md bg-white px-3 py-2 font-medium text-neutral-900"
        onClick={() => {
          setFailed(false);
          if (open) closeNativeViewer(consumer);
          else
            openNativeViewer(track, `${label} – Gelabber`).catch(() =>
              setFailed(true),
            );
        }}
      >
        {open ? "Fenster schließen" : "Im Fenster ansehen"}
      </button>
      {failed ? (
        <span role="status" className="text-xs">
          Das Videofenster konnte nicht geöffnet werden.
        </span>
      ) : null}
    </>
  );
}
