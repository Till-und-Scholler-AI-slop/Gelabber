// Local or remote video tile. srcObject is set in an effect so the
// preview can appear in the same frame as the stream, without SDP.

import { useCallback, useEffect, useRef, useState } from "react";

import { CollapseIcon, ExpandIcon } from "../components/Icons.tsx";
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
        <button type="button" onClick={props.onClose} autoFocus>
          Schließen <span aria-hidden="true">×</span>
        </button>
      </header>
      <VideoSurface {...props} expanded viewer onToggleExpand={undefined} />
    </dialog>
  );
}

function VideoSurface({
  stream,
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
      {!stream ? (
        <div className="absolute inset-0 flex items-center justify-center text-sm text-neutral-400 dark:text-neutral-500">
          {label}
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
    </figure>
  );
}
