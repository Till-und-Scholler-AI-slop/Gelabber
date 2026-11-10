import { Link } from "@tanstack/react-router";
import { useEffect, useId, useRef, useState } from "react";

import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { voice as texts } from "../i18n/catalogs/voice.ts";
import { voiceSettings } from "../i18n/catalogs/voiceSettings.ts";
import { compareNames } from "../i18n/format.ts";
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
import { useTooltip } from "./ui/Tooltip.tsx";
import { useLastOpen, usePresence } from "./ui/usePresence.ts";
import { useMediaQuery } from "./useMediaQuery.ts";
import { UserVolumeControl } from "./UserVolumeControl.tsx";
import { useCapabilities } from "../voice/capabilities.ts";
import { useCallShortcuts } from "../voice/callShortcuts.ts";
import { ConnectionStatus } from "../voice/ConnectionStatus.tsx";
import { CallSound } from "../voice/QuickCallSettings.tsx";
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
import { sharesSourceAudio, useMediaSettings } from "../voice/settings.ts";
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
  const t = useT(texts);
  const tc = useT(common);
  const dockRef = useRef<HTMLElement>(null);
  const session = useVoice();
  const sourceAudioShare = useMediaSettings((s) => s.sourceAudioShare);
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
    sharesSound:
      joined !== null &&
      capable.appAudio &&
      sharesSourceAudio({ sourceAudioShare }),
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
        aria-label={t("dock.label")}
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
              server={server}
            />
            <LeaveButton />
          </div>
        ) : null}
        {lost ? (
          <div className="voice-session-call voice-session-lost">
            <div className="voice-session-head" data-connection="failed">
              <WarningIcon size={16} className="voice-session-signal" />
              <span className="voice-session-link">
                <strong>{t("call.lost")}</strong>
                <span>{lost.channelName}</span>
              </span>
            </div>
            <Button
              onClick={() =>
                joinVoice({
                  serverId: lost.serverId,
                  channelId: lost.channelId,
                  channelName: lost.channelName,
                })
              }
            >
              {tc("tryAgain")}
            </Button>
            <IconButton
              label={tc("close")}
              icon={<CloseIcon size={16} />}
              onClick={() => dismissLostCall()}
            />
          </div>
        ) : null}
        {joined && pushToTalk && touch ? <HoldToTalk /> : null}
        {voice.playbackBlocked && !voice.deafened ? (
          <div role="status" className="voice-session-playback">
            <span>{t("dock.playbackBlocked")}</span>
            <button
              type="button"
              onClick={() => retryPlayback()}
              className="voice-control voice-room-primary"
            >
              {t("dock.startPlayback")}
            </button>
          </div>
        ) : notice ? (
          <div role="status" className="voice-session-notice">
            <WarningIcon size={15} />
            <span>{notice.text}</span>
            <IconButton
              label={t("dock.dismissNotice")}
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
 * words beside its quality as bars, one button that opens the details, and
 * the place below it, a link back to the channel. */
function CallStatus({
  voice,
  server,
  joined,
}: {
  voice: VoiceState;
  server: ServerDetail | undefined;
  joined: { serverId: string; channelId: string };
}) {
  const t = useT(texts);
  const tc = useT(common);
  const channel = voice.channelName ?? t("call.voiceChannel");
  const where = server?.name ? `${channel} · ${server.name}` : channel;
  const state =
    voice.connection === "connected"
      ? t("call.connected")
      : voice.connection === "reconnecting"
        ? t("call.reconnecting")
        : voice.connection === "failed"
          ? t("call.lost")
          : t("call.connecting");
  return (
    <div className="voice-session-head" data-connection={voice.connection}>
      <div className="voice-session-where">
        <ConnectionButton connection={voice.connection} state={state} />
        <Link
          to="/s/$serverId/c/$channelId"
          params={joined}
          className="voice-session-link"
          aria-label={`${state}: ${where}`}
        >
          <span>{where}</span>
        </Link>
      </div>
      {voice.connection === "failed" ? (
        <Button
          onClick={() =>
            joinVoice({
              serverId: joined.serverId,
              channelId: joined.channelId,
              channelName: channel,
            })
          }
        >
          {tc("tryAgain")}
        </Button>
      ) : null}
    </div>
  );
}

/** The state word with one to four bars from the latest measurement, or
 * the state's icon before it connects; pressing it shows the connection at
 * a glance, with the volumes and the way to the full diagnostics. */
function ConnectionButton({
  connection,
  state,
}: {
  connection: VoiceState["connection"];
  state: string;
}) {
  const t = useT(texts);
  const sample = useVoiceDiagnostics((s) => s.latest?.voice ?? null);
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLButtonElement>(null);
  const quality = callQuality(sample);
  const label = quality ? callQualitySummary(quality) : t("quality.measuring");
  const tip = useTooltip({ label, disabled: open });
  return (
    <>
      <button
        {...tip.trigger}
        ref={anchor}
        type="button"
        className="voice-session-status"
        aria-label={t("quality.showDetails", { summary: label })}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {connection === "connected" ? (
          <span
            className="voice-quality-bars"
            data-level={quality?.level ?? 0}
            aria-hidden="true"
          >
            {[1, 2, 3, 4].map((bar) => (
              <span key={bar} />
            ))}
          </span>
        ) : connection === "failed" ? (
          <WarningIcon size={16} className="voice-session-signal" />
        ) : (
          <SignalIcon size={16} className="voice-session-signal" />
        )}
        <strong>{state}</strong>
      </button>
      {tip.tooltip}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("quality.popoverLabel")}
        placement="top-start"
        className="voice-connection-popover"
      >
        <ConnectionDetails onDone={() => setOpen(false)} />
      </Popover>
    </>
  );
}

/** The "Verbindung" popover's content: the connection in short, what goes
 * out and comes in, the call's volumes (the phone's way to them without the
 * call bar's gear) and the ways to the settings and the diagnostics. */
export function ConnectionDetails({ onDone }: { onDone: () => void }) {
  const t = useT(texts);
  const ts = useT(voiceSettings);
  const volumeId = useId();
  const sample = useVoiceDiagnostics((s) => s.latest?.voice ?? null);
  const watch = useVoiceDiagnostics((s) => s.latest?.watch ?? null);
  const openSettings = useMediaSettings((s) => s.openDialog);
  const sent = directionTotal([sample], "send");
  const received = directionTotal([sample, watch], "recv");
  const show = (section: "audio" | "diagnostics") => {
    onDone();
    openSettings(section);
  };
  return (
    <>
      <ConnectionStatus compact />
      <dl className="connection-io">
        <div>
          <dt>
            <span className="connection-dir" aria-hidden="true">
              ↑
            </span>
            {t("quality.sending")}
          </dt>
          <dd>{sent === null ? "–" : formatBps(sent)}</dd>
        </div>
        <div>
          <dt>
            <span className="connection-dir" aria-hidden="true">
              ↓
            </span>
            {t("quality.receiving")}
          </dt>
          <dd>{received === null ? "–" : formatBps(received)}</dd>
        </div>
      </dl>
      <section className="connection-volume" aria-labelledby={volumeId}>
        <h3 id={volumeId}>{ts("quick.volume")}</h3>
        <CallSound />
      </section>
      <div className="connection-actions">
        <Button size="sm" variant="ghost" onClick={() => show("audio")}>
          {t("quality.settings")}
        </Button>
        {/* Focus starts on the way to the details (Popover.tsx). */}
        <Button size="sm" autoFocus onClick={() => show("diagnostics")}>
          {t("quality.allDetails")}
        </Button>
      </div>
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
  const t = useT(texts);
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
        name: members.get(id)?.name ?? t("people.someone"),
        url: members.get(id)?.avatar_url ?? null,
        speaking: speaking[id] === true && !silenced,
      };
    })
    .sort((a, b) => compareNames(a.name, b.name));
  const names = people.map((person) => person.name).join(", ");
  const tip = useTooltip({ label: names, disabled: open });
  if (people.length === 0) return null;
  const shown = [
    ...people.filter((person) => person.speaking),
    ...people.filter((person) => !person.speaking),
  ]
    .slice(0, SHOWN_PEOPLE)
    .sort((a, b) => compareNames(a.name, b.name));
  const more = people.length - shown.length;
  return (
    <>
      <button
        {...tip.trigger}
        ref={anchor}
        type="button"
        className="voice-session-people"
        aria-label={t("people.label", { names })}
        aria-haspopup="dialog"
        aria-expanded={open}
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
      {tip.tooltip}
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchor={anchor}
        label={t("people.title")}
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
                  {person.id === self
                    ? t("people.you", { name: person.name })
                    : person.name}
                </strong>
                {person.speaking ? <small>{t("people.speaking")}</small> : null}
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
  const t = useT(texts);
  const tip = useTooltip({ label: t("call.leave") });
  return (
    <>
      <button
        {...tip.trigger}
        type="button"
        onClick={() => leaveVoice()}
        aria-label={t("call.leave")}
        className="voice-control voice-control-compact voice-control-leave"
      >
        <HangUpIcon size={18} />
        <span className="voice-control-text">{t("call.leave")}</span>
      </button>
      {tip.tooltip}
    </>
  );
}
