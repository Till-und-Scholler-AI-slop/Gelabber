import { Link } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";

import { pageZoom } from "../interface/zoom.ts";
import { can } from "../servers/permissions.ts";
import { useServer } from "../servers/queries.ts";
import {
  dismissLostCall,
  joinVoice,
  leaveVoice,
  retryPlayback,
  useVoice,
  type VoiceState,
} from "../voice/session.ts";
import { HoldToTalk, VoiceControls } from "./VoiceControls.tsx";
import { CloseIcon, HangUpIcon, SignalIcon, WarningIcon } from "./Icons.tsx";
import { Button } from "./ui/Button.tsx";
import { IconButton } from "./ui/IconButton.tsx";
import { Popover } from "./ui/Popover.tsx";
import { useLastOpen, usePresence } from "./ui/usePresence.ts";
import { useMediaQuery } from "./useMediaQuery.ts";
import { UserVolumeControl } from "./UserVolumeControl.tsx";
import { useCapabilities } from "../voice/capabilities.ts";
import { useCallShortcuts } from "../voice/callShortcuts.ts";
import { ConnectionStatus } from "../voice/ConnectionStatus.tsx";
import { directionTotal } from "../voice/connectionSummary.ts";
import {
  callQuality,
  callQualitySummary,
  formatBps,
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
  const session = useVoice();
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
  const sourceAudioApp = useMediaSettings((s) => s.sourceAudioApp);
  const capable = useCapabilities();
  const pushToTalk = usePushToTalk((s) => s.active);
  const touch = useMediaQuery("(pointer: coarse)");
  const visible =
    session.status === "joined" ||
    session.playbackBlocked ||
    session.lostCall !== null;
  // Leaving, the bar slides away as it was, not emptied by the leave.
  const [voice] = useLastOpen(visible, [session] as const);
  const presence = usePresence(visible, dockRef);
  const { data: server } = useServer(voice.serverId ?? undefined);
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  useCallShortcuts(session.status === "joined");
  const lost = voice.status !== "joined" ? voice.lostCall : null;
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
  const announcement = <CallAnnouncement voice={session} />;
  if (!presence.mounted) return announcement;

  // Stream sound only matters while someone's Live is playing here.
  const streamSound =
    joined !== null &&
    Object.values(voice.sourceSubscriptions).some((source) => source.s);
  const notice = notices.find((candidate) => !dismissed.has(candidate.key));

  return (
    <>
      {announcement}
      <section
        ref={dockRef}
        aria-label="Aktive Medien"
        data-state={presence.state}
        inert={!visible}
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
            <VoiceControls
              canLive={can(server, "go_live")}
              streamSound={streamSound}
            />
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

/** One to four bars from the latest measurement; pressing them shows the
 * connection at a glance, with the way to the full diagnostics. */
function QualityBars() {
  const sample = useVoiceDiagnostics((state) => state.latest?.voice ?? null);
  const watch = useVoiceDiagnostics((state) => state.latest?.watch ?? null);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const quality = callQuality(sample);
  const label = quality
    ? callQualitySummary(quality)
    : "Verbindung: wird gemessen …";
  const sent = directionTotal([sample], "send");
  const received = directionTotal([sample, watch], "recv");
  const show = (section: "audio" | "diagnostics") => {
    setOpen(false);
    openSettings(section);
  };
  return (
    <>
      <button
        ref={anchor}
        type="button"
        className="voice-quality-bars"
        data-level={quality?.level ?? 0}
        aria-label={`${label} – Details anzeigen`}
        aria-haspopup="dialog"
        aria-expanded={open}
        title={label}
        onClick={() => setOpen((value) => !value)}
      >
        {[1, 2, 3, 4].map((bar) => (
          <span key={bar} aria-hidden="true" />
        ))}
      </button>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label="Verbindung"
        placement="top-start"
        className="voice-connection-popover"
      >
        <ConnectionStatus compact />
        <dl className="connection-io">
          <div>
            <dt>
              <span className="connection-dir" aria-hidden="true">
                ↑
              </span>
              Du sendest
            </dt>
            <dd>{sent === null ? "–" : formatBps(sent)}</dd>
          </div>
          <div>
            <dt>
              <span className="connection-dir" aria-hidden="true">
                ↓
              </span>
              Du empfängst
            </dt>
            <dd>{received === null ? "–" : formatBps(received)}</dd>
          </div>
        </dl>
        <div className="connection-actions">
          <Button size="sm" variant="ghost" onClick={() => show("audio")}>
            Einstellungen
          </Button>
          {/* Focus starts on the way to the details (Popover.tsx). */}
          <Button size="sm" autoFocus onClick={() => show("diagnostics")}>
            Alle Details
          </Button>
        </div>
      </Popover>
    </>
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
