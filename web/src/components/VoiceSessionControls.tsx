import { Link } from "@tanstack/react-router";
import { useEffect, useId, useRef, type CSSProperties } from "react";

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
  SpeakerOffIcon,
} from "./Icons.tsx";
import { useCapabilities } from "../voice/capabilities.ts";
import { EMPTY_OCCUPANCY, liveOf, useVoiceRoster } from "../voice/roster.ts";
import { useSpeakingUsers } from "../voice/speaking.ts";
import type { ServerDetail } from "../servers/types.ts";
import { Avatar } from "./Avatar.tsx";
import { useUserId } from "../auth/scope.ts";
import { isDesktopApp } from "../voice/native/bridge.ts";
import {
  MAX_PLAYBACK_VOLUME,
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
  const ids = useId();
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
  const self = useUserId();
  const liveHolder = useVoiceRoster((s) =>
    voice.serverId && voice.channelId
      ? liveOf(s.live, voice.serverId, voice.channelId)
      : null,
  );
  // Someone else's Live in this channel: Go Live says whose instead of
  // doing nothing.
  const liveTakenBy =
    liveHolder && liveHolder !== self && !voice.live
      ? (server?.members.find((member) => member.user_id === liveHolder)
          ?.name ?? "Jemand")
      : null;
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
            <CallPeople
              server={server}
              serverId={joined.serverId}
              channelId={joined.channelId}
            />
            {bar ? null : <LeaveButton />}
          </div>
          <VoiceControls
            canGoLive={can(server, "go_live")}
            liveTakenBy={liveTakenBy}
          />
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
      {voice.status !== "joined" || streamAudio ? (
        <div className="voice-session-mix">
          <div className="voice-session-mix-row">
            <label htmlFor={`${ids}-voices`}>
              {voice.status === "joined" ? "Stimmen" : "Lautstärke"}
            </label>
            <input
              id={`${ids}-voices`}
              type="range"
              min={0}
              max={MAX_PLAYBACK_VOLUME * 100}
              value={Math.round(volume * 100)}
              style={rangeFill(volume)}
              aria-label="Wiedergabe-Lautstärke"
              disabled={voice.deafened}
              onChange={(event) =>
                patch({ outputVolume: Number(event.target.value) / 100 })
              }
              className="lr-range"
            />
            {voice.status !== "joined" ? (
              <button
                type="button"
                aria-label="Voice-Einstellungen"
                title="Voice-Einstellungen"
                onClick={() => openSettings()}
                className="voice-control voice-control-compact"
              >
                <GearIcon size={16} />
              </button>
            ) : (
              <span aria-hidden="true" />
            )}
          </div>
          {streamAudio ? (
            <div className="voice-session-mix-row" aria-label="Stream-Ton">
              <label htmlFor={`${ids}-stream`}>Stream-Ton</label>
              <input
                id={`${ids}-stream`}
                type="range"
                min={0}
                max={MAX_PLAYBACK_VOLUME * 100}
                value={Math.round(sourceVolume * 100)}
                style={rangeFill(sourceVolume)}
                aria-label="Stream-Ton-Lautstärke"
                disabled={voice.deafened}
                onChange={(event) =>
                  patch({ sourceAudioVolume: Number(event.target.value) / 100 })
                }
                className="lr-range"
              />
              <button
                type="button"
                className={`voice-control voice-control-compact${sourceMuted ? " is-off" : ""}`}
                aria-pressed={sourceMuted}
                aria-label={sourceMuted ? "Stream-Ton an" : "Stream-Ton aus"}
                title={sourceMuted ? "Stream-Ton an" : "Stream-Ton aus"}
                onClick={() => patch({ sourceAudioMuted: !sourceMuted })}
              >
                {sourceMuted ? (
                  <SpeakerOffIcon size={16} />
                ) : (
                  <SpeakerIcon size={16} />
                )}
              </button>
            </div>
          ) : null}
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

const SHOWN_PEOPLE = 3;

/** Who is in the call, at a glance: whoever speaks is always among the
 * avatars shown, the rest keep their alphabetical place. */
function CallPeople({
  server,
  serverId,
  channelId,
}: {
  server: ServerDetail | undefined;
  serverId: string;
  channelId: string;
}) {
  const voice = useVoice();
  const self = useUserId();
  const occupancy = useVoiceRoster(
    (s) => s.byServer[serverId] ?? EMPTY_OCCUPANCY,
  );
  const speaking = useSpeakingUsers(serverId, channelId);
  const members = new Map(
    (server?.members ?? []).map((member) => [member.user_id, member]),
  );
  const people = Object.entries(occupancy)
    .filter(([, flags]) => flags.channelId === channelId)
    .map(([id, flags]) => {
      const silenced =
        id === self
          ? voice.muted || voice.deafened
          : flags.muted || flags.deafened;
      return {
        id,
        name: members.get(id)?.name ?? "Jemand",
        url: members.get(id)?.avatar_url ?? null,
        speaking: speaking[id] === true && !silenced,
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name, "de"));
  if (people.length === 0) return null;
  const shown = [
    ...people.filter((person) => person.speaking),
    ...people.filter((person) => !person.speaking),
  ]
    .slice(0, SHOWN_PEOPLE)
    .sort((a, b) => a.name.localeCompare(b.name, "de"));
  const more = people.length - shown.length;
  return (
    <span
      className="voice-session-people"
      title={people.map((person) => person.name).join(", ")}
      aria-hidden="true"
    >
      {shown.map((person) => (
        <Avatar
          key={person.id}
          name={person.name}
          url={person.url}
          className={person.speaking ? "is-speaking" : ""}
        />
      ))}
      {more > 0 ? (
        <span className="voice-session-people-more">+{more}</span>
      ) : null}
    </span>
  );
}

/** Filled share of a 0–200 % `.lr-range` playback slider (WebKit has no
 * progress pseudo). */
function rangeFill(value: number): CSSProperties {
  return {
    "--lr-range-value": `${Math.round((value / MAX_PLAYBACK_VOLUME) * 100)}%`,
  } as CSSProperties;
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
