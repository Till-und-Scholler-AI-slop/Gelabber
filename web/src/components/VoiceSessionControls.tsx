import { Link } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState, type CSSProperties } from "react";

import { pageZoom } from "../interface/zoom.ts";
import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import {
  dismissLostCall,
  joinVoice,
  leaveVoice,
  retryPlayback,
  stopWatching,
  useVoice,
  type VoiceState,
} from "../voice/session.ts";
import { HoldToTalk, VoiceControls } from "./VoiceControls.tsx";
import {
  CloseIcon,
  GearIcon,
  HangUpIcon,
  SignalIcon,
  SpeakerIcon,
  SpeakerOffIcon,
  WarningIcon,
} from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { Popover } from "./ui/Popover.tsx";
import { useMediaQuery } from "./useMediaQuery.ts";
import { UserVolumeControl } from "./UserVolumeControl.tsx";
import { useCapabilities } from "../voice/capabilities.ts";
import { useCallShortcuts } from "../voice/callShortcuts.ts";
import {
  callQuality,
  callQualitySummary,
  useVoiceDiagnostics,
} from "../voice/diagnostics.ts";
import { usePushToTalk } from "../voice/pushToTalk.ts";
import { EMPTY_OCCUPANCY, useVoiceRoster } from "../voice/roster.ts";
import { useSpeakingUsers } from "../voice/speaking.ts";
import type { ServerDetail } from "../servers/types.ts";
import { Avatar } from "./Avatar.tsx";
import { useUserId } from "../auth/scope.ts";
import { isDesktopApp } from "../voice/native/bridge.ts";
import {
  MAX_PLAYBACK_VOLUME,
  sharesSourceAudio,
  sourceAudioCarriesCall,
  useMediaSettings,
} from "../voice/settings.ts";
import {
  callAnnouncement,
  dockNotices,
  type CallMoment,
} from "../voice/callBar.ts";
import "../voice/room.css";

/** Active sessions stay controllable independently of the currently open
 * route: one bar at the bottom edge, after the sidebar on wide workspaces
 * and across the screen on phones and pages without a sidebar. */
export function VoiceSessionControls() {
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
  const pushToTalk = usePushToTalk((s) => s.active);
  const touch = useMediaQuery("(pointer: coarse)");
  const { data: server } = useServer(voice.serverId ?? undefined);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  useCallShortcuts(voice.status === "joined");
  const lost = voice.status !== "joined" ? voice.lostCall : null;
  const visible =
    voice.status === "joined" ||
    voice.watching ||
    voice.playbackBlocked ||
    lost !== null;
  useEffect(() => {
    const dock = dockRef.current;
    if (!visible || !dock) return;
    const update = () =>
      document.documentElement.style.setProperty(
        "--lr-media-space",
        `${Math.ceil(dock.getBoundingClientRect().height / pageZoom(dock))}px`,
      );
    update();
    const observer = new ResizeObserver(update);
    observer.observe(dock);
    return () => {
      observer.disconnect();
      document.documentElement.style.removeProperty("--lr-media-space");
    };
  }, [visible]);
  const joined =
    voice.status === "joined" && voice.serverId && voice.channelId
      ? { serverId: voice.serverId, channelId: voice.channelId }
      : null;
  const notices = dockNotices(voice, {
    desktop: isDesktopApp(),
    appAudio: capable.appAudio,
    sharesSound:
      joined !== null &&
      capable.appAudio &&
      sharesSourceAudio({ sourceAudioShare }),
    carriesCall: sourceAudioCarriesCall({ sourceAudioShare, sourceAudioApp }),
    sourceAudioApp,
  });
  // A dismissed notice that went away may come back with the next share.
  const current = new Set(notices.map((notice) => notice.key));
  if ([...dismissed].some((key) => !current.has(key)))
    setDismissed(new Set([...dismissed].filter((key) => current.has(key))));
  const announcement = <CallAnnouncement voice={voice} />;
  if (!visible) return announcement;

  // Stream sound only matters while someone's Live is playing here.
  const streamAudio =
    voice.watching ||
    Object.values(voice.sourceSubscriptions).some(
      (source) => source.s || source.l,
    );
  const showMix = joined
    ? streamAudio
    : voice.watching || voice.playbackBlocked;
  const notice = notices.find((candidate) => !dismissed.has(candidate.key));

  return (
    <>
      {announcement}
      <section
        ref={dockRef}
        aria-label="Aktive Medien"
        className="voice-session-dock"
      >
        {joined ? (
          <div
            className={`voice-session-call${voice.connection === "failed" ? " is-failed" : ""}`}
          >
            <CallStatus voice={voice} server={server} joined={joined} />
            <CallPeople
              server={server}
              serverId={joined.serverId}
              channelId={joined.channelId}
            />
            <VoiceControls canLive={can(server, "go_live")} />
            <LeaveButton />
          </div>
        ) : null}
        {lost ? (
          <div className="voice-session-call voice-session-lost">
            <div className="voice-session-head" data-connection="failed">
              <WarningIcon size={16} className="voice-session-signal" />
              <span className="voice-session-link">
                <strong>Verbindung verloren</strong>
                <span>{lost.channelName}</span>
              </span>
            </div>
            <Button
              size="sm"
              onClick={() =>
                joinVoice({
                  serverId: lost.serverId,
                  channelId: lost.channelId,
                  channelName: lost.channelName,
                })
              }
            >
              Erneut versuchen
            </Button>
            <IconButton
              label="Schließen"
              size="sm"
              icon={<CloseIcon size={16} />}
              onClick={() => dismissLostCall()}
            />
          </div>
        ) : null}
        {joined && pushToTalk && touch ? <HoldToTalk /> : null}
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
        {showMix ? (
          <div className="voice-session-mix">
            <div className="voice-session-mix-row">
              <label htmlFor={`${ids}-voices`}>
                {joined ? "Stimmen" : "Lautstärke"}
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
              {joined ? (
                <span aria-hidden="true" />
              ) : (
                <button
                  type="button"
                  aria-label="Voice-Einstellungen"
                  title="Voice-Einstellungen"
                  onClick={() => openSettings()}
                  className="voice-control voice-control-compact"
                >
                  <GearIcon size={16} />
                </button>
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
                    patch({
                      sourceAudioVolume: Number(event.target.value) / 100,
                    })
                  }
                  className="lr-range"
                />
                <button
                  type="button"
                  className={`voice-control voice-control-compact${sourceMuted ? " is-off" : ""}`}
                  aria-pressed={sourceMuted}
                  aria-label="Stream-Ton stummschalten"
                  title={
                    sourceMuted
                      ? "Stream-Ton wieder einschalten"
                      : "Stream-Ton stummschalten"
                  }
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
        ) : notice ? (
          <div role="status" className="voice-session-notice">
            <WarningIcon size={15} />
            <span>{notice.text}</span>
            <IconButton
              label="Hinweis schließen"
              size="xs"
              icon={<CloseIcon size={14} />}
              onClick={() =>
                setDismissed((previous) => new Set(previous).add(notice.key))
              }
            />
          </div>
        ) : null}
      </section>
    </>
  );
}

/** What changed about the call, said once to a screen reader: a region
 * that is always there, so the end of a call is heard too. */
function CallAnnouncement({ voice }: { voice: VoiceState }) {
  const [seen, setSeen] = useState<CallMoment>(voice);
  const [message, setMessage] = useState("");
  if (
    seen.status !== voice.status ||
    seen.connection !== voice.connection ||
    seen.channelName !== voice.channelName
  ) {
    setSeen(voice);
    const next = callAnnouncement(seen, voice);
    if (next) setMessage(next);
  }
  return (
    <p role="status" className="sr-only">
      {message}
    </p>
  );
}

/** Where the call is and whether it carries: the connection's state in
 * words, and once connected, its quality as bars that open the details. */
function CallStatus({
  voice,
  server,
  joined,
}: {
  voice: VoiceState;
  server: ServerDetail | undefined;
  joined: { serverId: string; channelId: string };
}) {
  const channel = voice.channelName ?? "Sprachkanal";
  const where = server?.name ? `${channel} · ${server.name}` : channel;
  const state =
    voice.connection === "connected"
      ? "Verbunden"
      : voice.connection === "reconnecting"
        ? "Verbindung wird wiederhergestellt …"
        : voice.connection === "failed"
          ? "Verbindung verloren"
          : "Verbinde …";
  return (
    <div className="voice-session-head" data-connection={voice.connection}>
      {voice.connection === "connected" ? (
        <QualityBars />
      ) : voice.connection === "failed" ? (
        <WarningIcon size={16} className="voice-session-signal" />
      ) : (
        <SignalIcon size={16} className="voice-session-signal" />
      )}
      <Link
        to="/s/$serverId/c/$channelId"
        params={joined}
        className="voice-session-link"
        aria-label={`${state}: ${where}`}
      >
        <strong>{state}</strong>
        <span>{where}</span>
      </Link>
      {voice.connection === "failed" ? (
        <Button
          size="sm"
          onClick={() =>
            joinVoice({
              serverId: joined.serverId,
              channelId: joined.channelId,
              channelName: channel,
            })
          }
        >
          Erneut versuchen
        </Button>
      ) : null}
    </div>
  );
}

/** One to four bars from the latest measurement; pressing them opens the
 * diagnostics. */
function QualityBars() {
  const sample = useVoiceDiagnostics((state) => state.latest?.voice ?? null);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const quality = callQuality(sample);
  const label = quality
    ? callQualitySummary(quality)
    : "Verbindung: wird gemessen …";
  return (
    <button
      type="button"
      className="voice-quality-bars"
      data-level={quality?.level ?? 0}
      aria-label={`${label} – Details anzeigen`}
      title={label}
      onClick={() => openSettings("diagnostics")}
    >
      {[1, 2, 3, 4].map((bar) => (
        <span key={bar} aria-hidden="true" />
      ))}
    </button>
  );
}

const SHOWN_PEOPLE = 3;

/** Who is in the call, at a glance: whoever speaks is always among the
 * avatars shown, the rest keep their alphabetical place. Pressing them
 * lists everyone with their volume. */
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
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
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
  const names = people.map((person) => person.name).join(", ");
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="voice-session-people"
        aria-label={`Im Anruf: ${names}`}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={names}
        onClick={() => setOpen((value) => !value)}
      >
        {shown.map((person) => (
          <Avatar
            key={person.id}
            name={person.name}
            url={person.url}
            decorative
            className={person.speaking ? "is-speaking" : ""}
          />
        ))}
        {more > 0 ? (
          <span className="voice-session-people-more" aria-hidden="true">
            +{more}
          </span>
        ) : null}
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label="Im Anruf"
        placement="top-start"
        className="voice-call-people"
      >
        <ul>
          {people.map((person) => (
            <li key={person.id}>
              <div className="voice-call-person">
                <Avatar
                  name={person.name}
                  url={person.url}
                  decorative
                  className={person.speaking ? "is-speaking" : ""}
                />
                <strong>
                  {person.name}
                  {person.id === self ? " (du)" : ""}
                </strong>
                {person.speaking ? <small>spricht</small> : null}
              </div>
              {person.id !== self ? (
                <UserVolumeControl userId={person.id} name={person.name} />
              ) : null}
            </li>
          ))}
        </ul>
      </Popover>
    </>
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
      <span className="voice-control-text">Verlassen</span>
    </button>
  );
}
