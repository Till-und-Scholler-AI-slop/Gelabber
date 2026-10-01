import { Link } from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import { retryPlayback, stopWatching, useVoice } from "../voice/session.ts";
import { VoiceControls } from "./VoiceControls.tsx";
import { GearIcon } from "./Icons.tsx";
import { useMediaSettings } from "../voice/settings.ts";
import "../voice/room.css";

/** Active sessions stay controllable independently of the currently open route. */
export function VoiceSessionControls() {
  const dockRef = useRef<HTMLElement>(null);
  const voice = useVoice();
  const volume = useMediaSettings((s) => s.outputVolume);
  const patch = useMediaSettings((s) => s.patch);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const { data: server } = useServer(voice.serverId ?? undefined);
  const visible =
    voice.status === "joined" || voice.watching || voice.playbackBlocked;
  useEffect(() => {
    const dock = dockRef.current;
    if (!visible || !dock) return;
    const update = () =>
      document.documentElement.style.setProperty(
        "--lr-media-space",
        `${Math.ceil(dock.getBoundingClientRect().height) + 24}px`,
      );
    update();
    const observer = new ResizeObserver(update);
    observer.observe(dock);
    return () => {
      observer.disconnect();
      document.documentElement.style.removeProperty("--lr-media-space");
    };
  }, [visible]);
  if (!visible) return null;

  return (
    <section
      ref={dockRef}
      aria-label="Aktive Medien"
      className="voice-session-dock"
    >
      {voice.status === "joined" && voice.serverId && voice.channelId ? (
        <div>
          <Link
            to="/s/$serverId/c/$channelId"
            params={{ serverId: voice.serverId, channelId: voice.channelId }}
            className="voice-session-link"
          >
            Verbunden: {voice.channelName ?? "Voice"}
          </Link>
          <VoiceControls compact canGoLive={can(server, "go_live")} />
        </div>
      ) : null}
      {voice.watching && voice.watchServerId && voice.watchChannelId ? (
        <div className="voice-session-watch">
          <Link
            to="/s/$serverId/c/$channelId"
            params={{
              serverId: voice.watchServerId,
              channelId: voice.watchChannelId,
            }}
            className="voice-session-link"
          >
            Zuschauen: {voice.watchChannelName ?? "Live"}
          </Link>
          <button
            type="button"
            onClick={() => stopWatching()}
            className="voice-control voice-control-leave"
          >
            Nicht mehr zuschauen
          </button>
        </div>
      ) : null}
      {voice.status !== "joined" ? (
        <div className="voice-controls voice-controls-compact">
          <label className="voice-playback-volume">
            <span className="sr-only">Wiedergabe</span>
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
            <GearIcon size={16} />
          </button>
        </div>
      ) : null}
      {voice.playbackBlocked && !voice.deafened ? (
        <div role="status" className="voice-session-playback">
          <span>Die Tonwiedergabe ist blockiert.</span>
          <button
            type="button"
            onClick={() => retryPlayback()}
            className="voice-control"
          >
            Ton starten
          </button>
        </div>
      ) : null}
    </section>
  );
}
