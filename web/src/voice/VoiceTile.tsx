// Local or remote video tile. srcObject is set in an effect so the
// preview can appear in the same frame as the stream, without SDP.
//
// Two actions per tile: "Hervorheben" pins it to the room's spotlight,
// "Vollbild" gives it the screen (the element where the page may go
// fullscreen, the video's own player on an iPhone, the large view
// otherwise). Fullscreen and the large view hide the call bar, so they
// bring a strip of their own: mute, deafen, stream sound, stop watching
// and the way out. It hides after a few idle seconds.

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";

import {
  CollapseIcon,
  ExpandIcon,
  EyeOffIcon,
  MoreIcon,
  PinIcon,
  SpeakerIcon,
  SpeakerOffIcon,
} from "../components/Icons.tsx";
import { VoiceAudioToggles } from "../components/VoiceControls.tsx";
import { Button } from "../components/ui/Button.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Popover } from "../components/ui/Popover.tsx";
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
import { toggleMute, useVoice } from "./session.ts";
import { MAX_PLAYBACK_VOLUME, useMediaSettings } from "./settings.ts";
import {
  CONTROLS_IDLE_MS,
  FRAME_LOADING_MS,
  FRAME_STALLED_MS,
  fullscreenRoute,
} from "./stage.ts";
import "./viewer.css";

type VoiceTileProps = {
  stream: MediaStream | null;
  label: string;
  mirror?: boolean;
  screen?: boolean;
  live?: boolean;
  /** The camera's owner is speaking. */
  speaking?: boolean;
  /** Large, in the room's spotlight. */
  spotlight?: boolean;
  /** "Hervorheben": whether the picture is pinned to the spotlight. */
  pinned?: boolean;
  onTogglePin?: () => void;
  /** A press on the picture: the filmstrip swaps it into the spotlight. */
  onSelect?: () => void;
  /** False where the name is written next to the tile already. */
  caption?: boolean;
  sourceWatch?: { watching: boolean; toggle: () => void };
  sourceAudioNotice?: string;
  /** Watching this Live without being in the call: how to stop. */
  onStopWatching?: () => void;
};

export function VoiceTile(props: VoiceTileProps) {
  const [viewing, setViewing] = useState(false);
  return (
    <>
      <VideoSurface {...props} onViewer={() => setViewing(true)} />
      {viewing && <StreamViewer {...props} onClose={() => setViewing(false)} />}
    </>
  );
}

/** The large view: fullscreen where the page cannot have it. */
export function StreamViewer(props: VoiceTileProps & { onClose: () => void }) {
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
      aria-label={`Vollbild: ${props.label}`}
      onCancel={(event) => {
        event.preventDefault();
        props.onClose();
      }}
    >
      <VideoSurface
        {...props}
        viewer
        onExit={props.onClose}
        onTogglePin={undefined}
        onSelect={undefined}
      />
    </dialog>
  );
}

/** True for a while after `wake`, then false until the next one. */
function useWake(initial: boolean): [boolean, () => void] {
  const [awake, setAwake] = useState(initial);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const wake = useCallback(() => {
    setAwake(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setAwake(false), CONTROLS_IDLE_MS);
  }, []);
  useEffect(() => {
    if (initial)
      timer.current = setTimeout(() => setAwake(false), CONTROLS_IDLE_MS);
    return () => clearTimeout(timer.current);
  }, [initial]);
  return [awake, wake];
}

/** How long `key` has waited for its first picture. */
function useFrameWait(
  key: string,
  waiting: boolean,
): "idle" | "loading" | "stalled" {
  const [phase, setPhase] = useState<{
    key: string;
    phase: "loading" | "stalled";
  } | null>(null);
  useEffect(() => {
    if (!waiting) return;
    const loading = setTimeout(
      () => setPhase({ key, phase: "loading" }),
      FRAME_LOADING_MS,
    );
    const stalled = setTimeout(
      () => setPhase({ key, phase: "stalled" }),
      FRAME_STALLED_MS,
    );
    return () => {
      clearTimeout(loading);
      clearTimeout(stalled);
    };
  }, [key, waiting]);
  return waiting && phase?.key === key ? phase.phase : "idle";
}

/** Screens and Live fill a phone held sideways. Desktop browsers refuse the
 * lock, which is fine. */
function lockLandscape(lock: boolean): void {
  const orientation = screen.orientation as
    (ScreenOrientation & { lock?: (to: string) => Promise<void> }) | undefined;
  try {
    if (lock) void orientation?.lock?.("landscape").catch(() => {});
    else orientation?.unlock?.();
  } catch {
    // No orientation API here.
  }
}

function VideoSurface({
  stream: given,
  label,
  mirror,
  screen: isScreen,
  live,
  speaking,
  spotlight,
  pinned = false,
  onTogglePin,
  onSelect,
  caption = true,
  sourceWatch,
  sourceAudioNotice,
  onStopWatching,
  onViewer,
  onExit,
  viewer = false,
}: VoiceTileProps & {
  onViewer?: () => void;
  onExit?: () => void;
  viewer?: boolean;
}) {
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
  useSyncExternalStore(
    subscribeNativeViewers,
    nativeViewersVersion,
    nativeViewersVersion,
  );
  // Closed, a window lets go of the stream a moment later.
  const windowOpen = consumer !== undefined && nativeViewerOpen(consumer);
  const [painted, setPainted] = useState<NativeTrack | null>(null);
  const [framed, setFramed] = useState<MediaStream | null>(null);
  const picture =
    (stream !== null && framed === stream) ||
    (drawn !== null && painted === drawn);
  const limit = useSyncExternalStore(
    subscribeNativeVideoLimit,
    nativeVideoLimit,
    nativeVideoLimit,
  );
  const joined = useVoice((s) => s.status === "joined");
  const [windowFailed, setWindowFailed] = useState<NativeTrack | null>(null);
  const figureRef = useRef<HTMLElement>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [awake, wake] = useWake(viewer);
  const immersive = viewer || fullscreen;
  const wasFullscreen = useRef(false);
  const refocus = useRef(false);
  useEffect(() => {
    const update = () => {
      const figure = figureRef.current;
      const on = document.fullscreenElement === figure;
      const was = wasFullscreen.current;
      wasFullscreen.current = on;
      setFullscreen(on);
      if (on) wake();
      else if (was) {
        if (isScreen) lockLandscape(false);
        const active = document.activeElement;
        refocus.current =
          active === null ||
          active === document.body ||
          Boolean(figure?.contains(active));
      }
    };
    document.addEventListener("fullscreenchange", update);
    return () => document.removeEventListener("fullscreenchange", update);
  }, [isScreen, wake]);
  useEffect(() => {
    const figure = figureRef.current;
    // The button that asked for fullscreen makes way for the strip: the
    // picture takes the focus, and with it M and Space. Back out, the
    // button has it again.
    if (fullscreen) figure?.focus({ preventScroll: true });
    else if (refocus.current)
      figure
        ?.querySelector<HTMLElement>("[data-fullscreen-toggle]")
        ?.focus({ preventScroll: true });
    refocus.current = false;
  }, [fullscreen]);
  const ref = useRef<HTMLVideoElement>(null);
  const enterFullscreen = async () => {
    const figure = figureRef.current;
    const route = fullscreenRoute(
      document.fullscreenEnabled,
      ref.current as { webkitEnterFullscreen?: unknown } | null,
    );
    if (route === "element" && figure?.requestFullscreen) {
      try {
        await figure.requestFullscreen();
        if (isScreen) lockLandscape(true);
        return;
      } catch {
        // Refused (an embedding frame, a policy): the large view instead.
      }
    } else if (route === "video" && ref.current) {
      (
        ref.current as { webkitEnterFullscreen?: () => void }
      ).webkitEnterFullscreen?.();
      return;
    }
    onViewer?.();
  };
  const exitFullscreen = () => {
    if (document.fullscreenElement === figureRef.current)
      void document.exitFullscreen().catch(() => {});
    else onExit?.();
  };
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

  const blocked = stream !== null && blockedStream === stream;
  // A picture is due unless the app shows none in the page, it plays in its
  // own window, or the source is not watched.
  const due =
    !picture &&
    !blocked &&
    !(nativeVideo && !inPage) &&
    !inWindow &&
    !(sourceWatch && !sourceWatch.watching);
  const [reloads, setReloads] = useState(0);
  const wait = useFrameWait(
    `${stream?.id ?? drawn?.id ?? "none"}:${reloads}`,
    due,
  );
  const canReload = Boolean(sourceWatch?.watching || stream);
  const reload = () => {
    if (sourceWatch?.watching) {
      // Off and on again: the server sends the source afresh.
      sourceWatch.toggle();
      sourceWatch.toggle();
    } else {
      const el = ref.current;
      if (el && stream) {
        el.srcObject = null;
        el.srcObject = stream;
        play();
      }
    }
    setReloads((count) => count + 1);
  };
  const stopWatching = sourceWatch?.watching
    ? sourceWatch.toggle
    : onStopWatching;
  // Stream sound plays for someone else's screen or Live.
  const streamSound = Boolean(isScreen && (sourceWatch || onStopWatching));
  const fit = isScreen || spotlight || immersive;
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (!immersive) return;
    wake();
    const target = event.target as HTMLElement;
    const key = event.key.toLowerCase();
    // Space presses a focused button; M only gives way to typing.
    const taken =
      key === " "
        ? target.closest("input, select, textarea, button, a")
        : target.closest("select, textarea, input:not([type='range'])");
    if (
      joined &&
      (key === "m" || key === " ") &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !taken
    ) {
      event.preventDefault();
      toggleMute();
    }
  };
  // Under a finger the actions show on a first tap; that tap does not also
  // press one of them unseen.
  const shownOnPress = useRef(true);

  return (
    <figure
      ref={figureRef}
      aria-label={caption ? undefined : label}
      tabIndex={immersive ? -1 : undefined}
      data-awake={awake || undefined}
      onPointerMove={immersive ? wake : undefined}
      onPointerDown={(event) => {
        if (immersive) wake();
        else if (event.pointerType !== "mouse") {
          shownOnPress.current = awake;
          wake();
        }
      }}
      onClickCapture={(event) => {
        const hidden = !shownOnPress.current;
        shownOnPress.current = true;
        if (
          hidden &&
          (event.target as Element).closest(".voice-video-actions")
        ) {
          event.preventDefault();
          event.stopPropagation();
        }
      }}
      onFocus={immersive ? wake : undefined}
      onKeyDown={onKeyDown}
      onClick={
        onSelect
          ? (event) => {
              // The picture, not one of the buttons on it or a menu of it.
              const target = event.target as Element;
              if (
                event.currentTarget.contains(target) &&
                !target.closest("button, a, input")
              )
                onSelect();
            }
          : undefined
      }
      className={[
        "voice-video-tile",
        viewer ? "voice-video-viewer" : "",
        spotlight ? "is-spotlight" : "",
        onSelect ? "is-selectable" : "",
        speaking ? "is-speaking" : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {drawn ? (
        <NativeVideoCanvas
          key={drawn.id}
          track={drawn}
          onDoubleClick={() =>
            immersive ? exitFullscreen() : void enterFullscreen()
          }
          onPainted={(done) => setPainted(done ? drawn : null)}
          className={[
            "size-full",
            fit ? "object-contain" : "object-cover",
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
          onLoadedData={() => setFramed(stream)}
          onPlaying={() => setFramed(stream)}
          onDoubleClick={() =>
            immersive ? exitFullscreen() : void enterFullscreen()
          }
          className={[
            "size-full",
            fit ? "object-contain" : "object-cover",
            mirror ? "-scale-x-100" : "",
            stream ? "" : "opacity-0",
          ].join(" ")}
        />
      )}
      {blocked ? (
        <div className="voice-video-overlay">
          <p role="status">Die Videowiedergabe ist blockiert.</p>
          <Button size="sm" variant="primary" onClick={play}>
            Wiedergabe starten
          </Button>
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
      {/* A browser stream's first frame is a moment away: no name flashes
          up for it, only a spinner if it takes longer. */}
      {!picture && !blocked && (stream === null || wait !== "idle") ? (
        <div className="voice-video-placeholder">
          {wait === "idle" ? <span>{label}</span> : null}
          {wait === "loading" ? (
            <span role="status" className="voice-video-wait">
              <span className="voice-video-spinner" aria-hidden="true" />
              Video wird geladen …
            </span>
          ) : null}
          {wait === "stalled" ? (
            <>
              <span role="status">Kein Bild empfangen</span>
              {canReload ? (
                <Button size="sm" onClick={reload}>
                  Neu laden
                </Button>
              ) : null}
            </>
          ) : null}
          {nativeVideo && !inPage ? (
            <NativeVideo stream={given} label={label} />
          ) : null}
          {inWindow ? (
            <span role="status" className="text-xs">
              Läuft im eigenen Fenster.
            </span>
          ) : null}
          {inWindow && windowOpen ? (
            <Button
              size="sm"
              onClick={(event) => {
                // The button goes with the window. In the large view focus
                // stays in it, on its way out; on a tile it moves to the
                // menu the window was opened from.
                const button = event.currentTarget;
                (
                  button
                    .closest("dialog")
                    ?.querySelector<HTMLElement>("[data-viewer-close]") ??
                  button
                    .closest("figure")
                    ?.querySelector<HTMLElement>('[aria-haspopup="menu"]')
                )?.focus();
                closeNativeViewer(consumer);
              }}
            >
              Fenster schließen
            </Button>
          ) : null}
        </div>
      ) : null}
      {caption && !immersive ? (
        <figcaption className="voice-video-caption">{label}</figcaption>
      ) : null}
      {live ? <span className="voice-video-live">Live</span> : null}
      {drawn && limit ? (
        <span
          role="status"
          title={LIMIT_REASON[limit]}
          className={[
            "voice-video-limit absolute left-2",
            live ? "top-8" : "top-2",
          ].join(" ")}
        >
          Geringe Bildqualität
        </span>
      ) : null}
      {immersive ? (
        <>
          <WatchStrip
            label={label}
            joined={joined}
            streamSound={streamSound}
            autoFocus={viewer}
            onStopWatching={
              stopWatching
                ? () => {
                    exitFullscreen();
                    stopWatching();
                  }
                : undefined
            }
            onExit={exitFullscreen}
          />
        </>
      ) : (
        <>
          <div className="voice-video-scrim" aria-hidden="true" />
          <div className="voice-video-actions">
            {sourceWatch?.watching ? (
              <IconButton
                size="sm"
                label="Nicht mehr zuschauen"
                icon={<EyeOffIcon size={16} />}
                onClick={sourceWatch.toggle}
                className="voice-video-action"
              />
            ) : null}
            {onTogglePin ? (
              <IconButton
                size="sm"
                label="Hervorheben"
                pressed={pinned}
                icon={<PinIcon size={16} />}
                onClick={onTogglePin}
                className="voice-video-action"
              />
            ) : null}
            <IconButton
              size="sm"
              label="Vollbild"
              title="Vollbild (auch per Doppelklick)"
              data-fullscreen-toggle=""
              icon={<ExpandIcon size={16} />}
              onClick={() => void enterFullscreen()}
              className="voice-video-action"
            />
            {track && consumer !== undefined ? (
              <TileMenu label={label}>
                <OwnWindow
                  track={track}
                  label={label}
                  onFailed={(failed) => setWindowFailed(failed ? track : null)}
                />
              </TileMenu>
            ) : null}
          </div>
        </>
      )}
      {track && windowFailed === track ? (
        <p role="status" className="voice-fullscreen-error">
          Das Videofenster konnte nicht geöffnet werden.
        </p>
      ) : null}
    </figure>
  );
}

/** The tile's rarely used actions, behind "…". */
function TileMenu({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  return (
    <>
      <IconButton
        ref={anchor}
        size="sm"
        label="Weitere Aktionen"
        aria-haspopup="menu"
        aria-expanded={open}
        icon={<MoreIcon size={16} />}
        onClick={() => setOpen((value) => !value)}
        className="voice-video-action"
      />
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={label}
        role="menu"
        placement="bottom-end"
      >
        <div role="none" className="voice-menu" onClick={() => setOpen(false)}>
          {children}
        </div>
      </Popover>
    </>
  );
}

/** Mute, deafen, the stream's sound, stop watching and the way out, while
 * the call bar is out of sight. */
function WatchStrip({
  label,
  joined,
  streamSound,
  autoFocus,
  onStopWatching,
  onExit,
}: {
  label: string;
  joined: boolean;
  streamSound: boolean;
  autoFocus: boolean;
  onStopWatching: (() => void) | undefined;
  onExit: () => void;
}) {
  return (
    <div
      className="voice-watch-strip"
      role="group"
      aria-label={`Steuerung: ${label}`}
    >
      <strong className="voice-watch-strip-label">{label}</strong>
      {joined ? (
        <div className="voice-watch-strip-group">
          <VoiceAudioToggles />
        </div>
      ) : null}
      {streamSound ? <StreamVolume /> : null}
      {onStopWatching ? (
        <button
          type="button"
          className="voice-watch-button"
          onClick={onStopWatching}
        >
          <EyeOffIcon size={16} />
          <span>Nicht mehr zuschauen</span>
        </button>
      ) : null}
      <button
        type="button"
        className="voice-watch-button"
        data-viewer-close=""
        autoFocus={autoFocus}
        onClick={onExit}
      >
        <CollapseIcon size={16} />
        <span>Vollbild beenden</span>
      </button>
    </div>
  );
}

/** The stream sound's own volume, next to the picture it belongs to. */
function StreamVolume() {
  const volume = useMediaSettings((s) => s.sourceAudioVolume);
  const muted = useMediaSettings((s) => s.sourceAudioMuted);
  const deafened = useVoice((s) => s.deafened);
  const patch = useMediaSettings((s) => s.patch);
  return (
    <div className="voice-watch-strip-group voice-watch-volume">
      <button
        type="button"
        aria-pressed={muted}
        aria-label="Stream-Ton stummschalten"
        title={
          muted ? "Stream-Ton wieder einschalten" : "Stream-Ton stummschalten"
        }
        className={`voice-control voice-control-compact${muted ? " is-off" : ""}`}
        onClick={() => patch({ sourceAudioMuted: !muted })}
      >
        {muted ? <SpeakerOffIcon size={17} /> : <SpeakerIcon size={17} />}
      </button>
      <input
        type="range"
        min={0}
        max={MAX_PLAYBACK_VOLUME * 100}
        value={Math.round(volume * 100)}
        style={
          {
            "--lr-range-value": `${Math.round((volume / MAX_PLAYBACK_VOLUME) * 100)}%`,
          } as CSSProperties
        }
        aria-label="Stream-Ton-Lautstärke"
        aria-valuetext={`${Math.round(volume * 100)} %`}
        disabled={muted || deafened}
        onChange={(event) =>
          patch({ sourceAudioVolume: Number(event.target.value) / 100 })
        }
        className="lr-range"
      />
    </div>
  );
}

const videoFrames = () => hasNativeFeature("video-frames");

const LIMIT_REASON = {
  transport:
    "Der Server blockiert die schnelle Bildübertragung der Desktop-App (Content-Security-Policy). Das Bild bleibt deshalb klein.",
  renderer: "WebGL ist hier nicht verfügbar. Das Bild bleibt deshalb klein.",
};

/** Someone's stream in a window of its own instead of in its tiles: for a
 * second monitor or a tiling compositor. An entry of the tile's "…" menu. */
function OwnWindow({
  track,
  label,
  onFailed,
}: {
  track: NativeTrack;
  label: string;
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
  return (
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={open}
      className="voice-menu-item"
      onClick={() => {
        onFailed(false);
        if (open) closeNativeViewer(consumer);
        else
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
      <Button
        size="sm"
        variant="primary"
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
      </Button>
      {failed ? (
        <span role="status" className="text-xs">
          Das Videofenster konnte nicht geöffnet werden.
        </span>
      ) : null}
    </>
  );
}
