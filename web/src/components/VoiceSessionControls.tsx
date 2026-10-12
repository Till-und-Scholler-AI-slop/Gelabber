import { Link } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import {
  leaveVoice,
  retryPlayback,
  stopWatching,
  useVoice,
} from "../voice/session.ts";
import { VoiceControls } from "./VoiceControls.tsx";
import {
  CloseIcon,
  GearIcon,
  HangUpIcon,
  SignalIcon,
  SpeakerIcon,
} from "./Icons.tsx";
import { useCapabilities } from "../voice/capabilities.ts";
import { isDesktopApp } from "../voice/native/bridge.ts";
import {
  SOURCE_AUDIO_CARRIES_CALL,
  sharesSourceAudio,
  sourceAudioCarriesCall,
  sourceAudioScope,
  useMediaSettings,
} from "../voice/settings.ts";
import "../voice/room.css";

/** Active sessions stay controllable independently of the currently open
 * route. `bar` is the fixed bottom row (phones, pages without a sidebar);
 * `card` sits in the sidebar above the user panel on wide workspaces. */
export function VoiceSessionControls({
  variant = "bar",
}: {
  variant?: "bar" | "card";
}) {
  const dockRef = useRef<HTMLElement>(null);
  const voice = useVoice();
  const volume = useMediaSettings((s) => s.outputVolume);
  const sourceVolume = useMediaSettings((s) => s.sourceAudioVolume);
  const sourceMuted = useMediaSettings((s) => s.sourceAudioMuted);
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
  const sourceAudioApp = useMediaSettings((s) => s.sourceAudioApp);
  const patch = useMediaSettings((s) => s.patch);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const capable = useCapabilities();
  const { data: server } = useServer(voice.serverId ?? undefined);
  const visible =
    voice.status === "joined" || voice.watching || voice.playbackBlocked;
  const bar = variant === "bar";
  useEffect(() => {
    const dock = dockRef.current;
    if (!bar || !visible || !dock) return;
    // Hidden by CSS where the sidebar card takes over: height 0, no space.
    const update = () =>
      document.documentElement.style.setProperty(
        "--lr-media-space",
        `${Math.ceil(dock.getBoundingClientRect().height)}px`,
      );
    update();
    const observer = new ResizeObserver(update);
    observer.observe(dock);
    return () => {
      observer.disconnect();
      document.documentElement.style.removeProperty("--lr-media-space");
    };
  }, [bar, visible]);
  if (!visible) return null;

  const joined =
    voice.status === "joined" && voice.serverId && voice.channelId
      ? { serverId: voice.serverId, channelId: voice.channelId }
      : null;
  // Stream sound only matters while someone's screen or Live is playing here.
  const streamAudio =
    voice.watching ||
    Object.values(voice.sourceSubscriptions).some(
      (source) => source.s || source.l,
    );
  // The sound of the own share: why it is missing, and what the control
  // next to the share button will do.
  const desktop = isDesktopApp();
  const sharesSound =
    joined !== null &&
    capable.appAudio &&
    sharesSourceAudio({ sourceAudioShare });
  const notes = voice.sourceAudioNote;
  const failed = notes.s?.failed ?? notes.l?.failed;
  const silent = notes.s?.silent ?? notes.l?.silent;
  // Without a chosen application the desktop app sends the sound of all of
  // them with the one window that was picked, and it does so unasked. While
  // such a share runs, the dock says so next to the switch that ends it.
  const everyApplication =
    sourceAudioApp === "" &&
    (voice.sourceAudio.s === "sharing" || voice.sourceAudio.l === "sharing")
      ? sourceAudioScope({ sourceAudioApp })
      : null;
  // A browser hands sound over in its picker only: a share that started
  // without it stays without it.
  const nextShare =
    sharesSound &&
    !desktop &&
    ((voice.localScreen && voice.sourceAudio.s === "off") ||
      (voice.localLive && voice.sourceAudio.l === "off"));

  return (
    <section
      ref={dockRef}
      aria-label="Aktive Medien"
      className={bar ? "voice-session-dock" : "voice-session-card"}
    >
      {joined ? (
        <div className="voice-session-call">
          <div className="voice-session-head">
            <SignalIcon size={16} className="voice-session-signal" />
            <Link
              to="/s/$serverId/c/$channelId"
              params={joined}
              className="voice-session-link"
              aria-label={`Verbunden: ${voice.channelName ?? "Voice"}`}
            >
              <strong>Verbunden</strong>
              <span>{voice.channelName ?? "Voice"}</span>
            </Link>
            {bar ? null : <LeaveButton />}
          </div>
          <VoiceControls canGoLive={can(server, "go_live")} />
          {bar ? <LeaveButton /> : null}
        </div>
      ) : null}
      {voice.watching && voice.watchServerId && voice.watchChannelId ? (
        <div className="voice-session-watch">
          <span className="voice-session-live">Live</span>
          <Link
            to="/s/$serverId/c/$channelId"
            params={{
              serverId: voice.watchServerId,
              channelId: voice.watchChannelId,
            }}
            className="voice-session-link"
            aria-label={`Zuschauen: ${voice.watchChannelName ?? "Live"}`}
          >
            <strong>Du schaust zu</strong>
            <span>{voice.watchChannelName ?? "Live"}</span>
          </Link>
          <button
            type="button"
            onClick={() => stopWatching()}
            aria-label="Nicht mehr zuschauen"
            title="Nicht mehr zuschauen"
            className="voice-control voice-control-compact"
          >
            <CloseIcon size={17} />
          </button>
        </div>
      ) : null}
      {voice.status !== "joined" ? (
        <div className="voice-session-row">
          <label className="voice-playback-volume">
            <SpeakerIcon size={15} />
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(volume * 100)}
              aria-label="Wiedergabe-Lautstärke"
              disabled={voice.deafened}
              onChange={(event) =>
                patch({ outputVolume: Number(event.target.value) / 100 })
              }
              className="voice-volume-slider"
            />
          </label>
          <button
            type="button"
            aria-label="Voice-Einstellungen"
            title="Voice-Einstellungen"
            onClick={() => openSettings()}
            className="voice-control voice-control-compact"
          >
            <GearIcon size={17} />
          </button>
        </div>
      ) : null}
      {streamAudio && voice.status === "joined" ? (
        <div className="voice-session-row">
          <label className="voice-playback-volume">
            <span>Stimmen</span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(volume * 100)}
              aria-label="Wiedergabe-Lautstärke"
              disabled={voice.deafened}
              onChange={(event) =>
                patch({ outputVolume: Number(event.target.value) / 100 })
              }
              className="voice-volume-slider"
            />
          </label>
        </div>
      ) : null}
      {streamAudio ? (
        <div className="voice-session-row" aria-label="Stream-Ton">
          <button
            type="button"
            className="voice-control voice-control-compact"
            aria-pressed={sourceMuted}
            aria-label={sourceMuted ? "Stream-Ton an" : "Stream-Ton aus"}
            title={sourceMuted ? "Stream-Ton an" : "Stream-Ton aus"}
            onClick={() => patch({ sourceAudioMuted: !sourceMuted })}
          >
            <SpeakerIcon size={17} />
          </button>
          <label className="voice-playback-volume">
            <span>Stream-Ton</span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(sourceVolume * 100)}
              aria-label="Stream-Ton-Lautstärke"
              disabled={voice.deafened}
              onChange={(event) =>
                patch({ sourceAudioVolume: Number(event.target.value) / 100 })
              }
              className="voice-volume-slider"
            />
          </label>
        </div>
      ) : null}
      {(voice.sourceAudio.s === "unavailable" ||
        voice.sourceAudio.l === "unavailable") && (
        <p role="status" className="voice-source-audio-notice">
          {!desktop
            ? "Der Browser hat keinen Stream-Ton freigegeben."
            : !capable.appAudio
              ? "Diese Desktop-App kann keinen Ton von Anwendungen teilen."
              : failed
                ? `Der Stream-Ton konnte nicht aufgenommen werden (${failed}).`
                : "Der Stream-Ton konnte nicht aufgenommen werden."}{" "}
          Das Video läuft weiter.
        </p>
      )}
      {(voice.sourceAudio.s === "unsupported" ||
        voice.sourceAudio.l === "unsupported") && (
        <p role="status" className="voice-source-audio-notice">
          Dieser Medienserver unterstützt Stream-Ton noch nicht. Das Video läuft
          weiter.
        </p>
      )}
      {(voice.sourceAudio.s === "ended" || voice.sourceAudio.l === "ended") && (
        <p role="status" className="voice-source-audio-notice">
          Der Stream-Ton wurde beendet. Das Video läuft weiter.
        </p>
      )}
      {silent ? (
        <p role="status" className="voice-source-audio-notice">
          Von „{silent}“ kommt gerade kein Ton.
        </p>
      ) : null}
      {everyApplication ? (
        <p role="status" className="voice-source-audio-notice">
          Dein Stream sendet den {everyApplication}.
        </p>
      ) : null}
      {sharesSound &&
      sourceAudioCarriesCall({ sourceAudioShare, sourceAudioApp }) ? (
        <p role="status" className="voice-source-audio-notice">
          {SOURCE_AUDIO_CARRIES_CALL}
        </p>
      ) : null}
      {nextShare ? (
        <p role="status" className="voice-source-audio-notice">
          Der Stream-Ton kommt mit der nächsten Freigabe dazu.
        </p>
      ) : null}
      {voice.playbackBlocked && !voice.deafened ? (
        <div role="status" className="voice-session-playback">
          <span>Der Browser blockiert den Ton.</span>
          <button
            type="button"
            onClick={() => retryPlayback()}
            className="voice-control voice-room-primary"
          >
            Ton starten
          </button>
        </div>
      ) : null}
    </section>
  );
}

function LeaveButton() {
  return (
    <button
      type="button"
      onClick={() => leaveVoice()}
      aria-label="Verlassen"
      title="Verlassen"
      className="voice-control voice-control-compact voice-control-leave"
    >
      <HangUpIcon size={18} />
    </button>
  );
}
