// Voice channel body: join is a local state flip, ICE runs afterwards.
// Camera and screen tiles render local streams immediately; SFU publish
// is background work and never rides the chat socket.
//
// Lives take the stage above the people: one in the spotlight, the others
// in a filmstrip. A camera stays in its owner's place, so each person
// appears once, unless it is pinned to the spotlight. A Live is watched in
// the room: outside it, the room only says who is live.

import { useEffect, useRef, useState, type RefObject } from "react";

import { can } from "../servers/permissions.ts";
import type { Member, ServerDetail } from "../servers/types.ts";
import { VoiceStateIcons } from "../components/VoiceStateIcons.tsx";
import { MemberProfileDialog } from "../components/MemberProfileDialog.tsx";
import { Avatar } from "../components/Avatar.tsx";
import { MicIcon, MoreIcon, SpeakerIcon } from "../components/Icons.tsx";
import { Button } from "../components/ui/Button.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Popover } from "../components/ui/Popover.tsx";
import { POPOVER_SHEET_QUERY } from "../components/ui/popoverPosition.ts";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import { useSession } from "../auth/session.ts";
import { joinVoice, useVoice, toggleSourceWatch } from "./session.ts";
import { isDesktopApp } from "./native/bridge.ts";
import {
  EMPTY_LIVE_PEOPLE,
  EMPTY_OCCUPANCY,
  useVoiceRoster,
} from "./roster.ts";
import { useSpeakingUsers } from "./speaking.ts";
import { VoiceTile } from "./VoiceTile.tsx";
import {
  LocallyMutedIcon,
  UserVolumeControl,
} from "../components/UserVolumeControl.tsx";
import { userVolumeOf, useUserVolumes } from "./userVolume.ts";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { QuickCallSettingsButton } from "./QuickCallSettings.tsx";
import { roomSummary } from "./roomSummary.ts";
import { arrangeStage, createLongPress, type StageKind } from "./stage.ts";
import "./room.css";

type StageTile = {
  id: string;
  kind: StageKind;
  /** The track of a Live: a screen, or an older client's Go Live. */
  source?: "s" | "l";
  userId: string;
  stream: MediaStream | null;
  name: string;
  mirror?: boolean;
  speaking?: boolean;
};

/** Empty seats before joining: a room with nobody in it shows a few. */
const FREE_SEATS_EMPTY = 3;

export function VoiceRoom({
  server,
  channelId,
  channelName,
  named = false,
}: {
  server: ServerDetail;
  channelId: string;
  channelName: string;
  /** Show the room's name: where no page heading names it already. */
  named?: boolean;
}) {
  const allowed = can(server, "join_voice");
  const voice = useVoice();
  const user = useSession((s) => s.user);
  const me = user?.id;
  const roster = useVoiceRoster(
    (s) => s.byServer[server.id] ?? EMPTY_OCCUPANCY,
  );
  const livePeople = useVoiceRoster(
    (s) => s.live[server.id]?.[channelId] ?? EMPTY_LIVE_PEOPLE,
  );
  const here =
    voice.status === "joined" &&
    voice.serverId === server.id &&
    voice.channelId === channelId;
  const speakingUsers = useSpeakingUsers(server.id, channelId);
  const roomKey = `${server.id}/${channelId}`;
  const [testingMicrophone, setTestingMicrophone] = useState<string | null>(
    null,
  );
  const [profile, setProfile] = useState<{
    serverId: string;
    memberId: string;
  } | null>(null);
  const [pinned, setPinned] = useState<string | null>(null);
  const members = new Map(
    server.members.map((member) => [member.user_id, member]),
  );
  const occupants = Object.entries(roster).filter(
    ([id, flags]) => flags.channelId === channelId && (id !== me || here),
  );

  // Local join is immediate, before the server roster echo arrives.
  if (here && me && !occupants.some(([id]) => id === me)) {
    occupants.push([
      me,
      { channelId, muted: voice.muted, deafened: voice.deafened },
    ]);
  }

  const onJoin = () => {
    if (!allowed) return;
    setTestingMicrophone(null);
    joinVoice({ serverId: server.id, channelId, channelName });
  };
  const openProfile = (memberId: string) =>
    setProfile({ serverId: server.id, memberId });

  const nameOf = (id: string) =>
    id === me
      ? `${members.get(id)?.name ?? user?.name ?? "Du"} (du)`
      : (members.get(id)?.name ?? "Mitglied");
  const shares: StageTile[] = [];
  const cameras = new Map<string, StageTile>();
  const speaks = (id: string, flags: { muted: boolean; deafened: boolean }) =>
    speakingUsers[id] === true &&
    !(id === me && here
      ? voice.muted || voice.deafened
      : flags.muted || flags.deafened);
  // Pictures only inside: a Live is watched in the room.
  for (const [id, flags] of here ? occupants : []) {
    const pubs = voice.participants[id]?.pubs ?? [];
    const self = id === me;
    const name = nameOf(id);
    const cameraOn = self ? voice.camera : pubs.includes("v");
    const screen = self ? voice.sharing : pubs.includes("s");
    // An older client's Go Live shows like any other Live.
    const goLive = self ? voice.live : pubs.includes("l");
    for (const [source, on, stream] of [
      ["s", screen, self ? voice.localScreen : voice.remote[id]?.s],
      ["l", goLive, self ? voice.localLive : voice.remote[id]?.l],
    ] as const) {
      if (!on) continue;
      shares.push({
        id: `${id}-${source}`,
        kind: "live",
        source,
        userId: id,
        stream: stream ?? null,
        name: `${name} — Live`,
      });
    }
    if (cameraOn) {
      cameras.set(id, {
        id: `${id}-v`,
        kind: "camera",
        userId: id,
        stream: self ? voice.localCamera : (voice.remote[id]?.v ?? null),
        name,
        mirror: self,
        speaking: speaks(id, flags),
      });
    }
  }
  // Whoever went live first keeps the spotlight when another joins in.
  const order = Object.keys(livePeople);
  const rank = (tile: StageTile) => {
    const at = order.indexOf(tile.userId);
    return at === -1 ? order.length : at;
  };
  shares.sort((a, b) => rank(a) - rank(b));
  const isLive = (id: string) =>
    Boolean(livePeople[id]) ||
    shares.some((tile) => tile.userId === id) ||
    (id === me && here && (voice.sharing || voice.live));

  const sourceProps = (tile: StageTile) => {
    if (!tile.source) return {};
    const kind = tile.source;
    if (tile.userId === me && here) {
      const status = voice.sourceAudio[kind];
      // The reason stands next to the call controls. A picker withholds
      // sound in a browser only; the desktop app captures it by itself.
      const silent = voice.sourceAudioNote[kind]?.silent;
      return {
        sourceAudioNotice:
          status === "unavailable"
            ? isDesktopApp()
              ? "Stream-Ton nicht aufgenommen · Video läuft weiter"
              : "Kein Stream-Ton freigegeben · Video läuft weiter"
            : status === "ended"
              ? "Stream-Ton beendet · Video läuft weiter"
              : status === "sharing"
                ? silent
                  ? `Kein Ton von „${silent}“`
                  : "Ton wird mitgesendet"
                : undefined,
      };
    }
    if (here && voice.sourceWatchSupported)
      return {
        sourceWatch: {
          watching: voice.sourceSubscriptions[tile.userId]?.[kind] === true,
          toggle: () => toggleSourceWatch(tile.userId, kind),
        },
      };
    return {};
  };

  const { spotlight, filmstrip } = arrangeStage(
    shares,
    [...cameras.values()],
    pinned,
  );
  const togglePin = (id: string) =>
    setPinned((current) => (current === id ? null : id));
  const stageTiles = spotlight ? [spotlight, ...filmstrip] : [];
  const liveOn = occupants.some(([id]) => isLive(id));
  const others = occupants
    .filter(([id]) => id !== me)
    .map(([id]) => members.get(id)?.name ?? "Jemand");
  const othersLive = occupants
    .filter(([id]) => id !== me && isLive(id))
    .map(([id]) => members.get(id)?.name ?? "Jemand");
  const elsewhere = voice.status === "joined" && !here;
  const freeSeats =
    here || !allowed ? 0 : occupants.length === 0 ? FREE_SEATS_EMPTY : 1;

  return (
    <section
      className="voice-room"
      aria-label={channelName}
      data-joined={here || undefined}
      data-page={!named || undefined}
    >
      {named ? (
        <h2 className="voice-room-name">
          {channelName}
          {liveOn ? <span className="voice-room-live-badge">Live</span> : null}
        </h2>
      ) : null}
      <p className="voice-room-summary">
        {roomSummary(others, { here, canJoin: allowed, live: othersLive })}
      </p>
      {stageTiles.length > 0 ? (
        <div
          className="voice-room-stage"
          role="region"
          aria-label="Übertragungen"
        >
          {/* One element per picture, in a fixed order: the spotlight is a
              place on the stage, so swapping never rebuilds a video. */}
          {[...shares, ...cameras.values()]
            .filter((tile) => stageTiles.includes(tile))
            .map((tile) => {
              const inSpotlight = tile === spotlight;
              return (
                <div
                  key={tile.id}
                  className={
                    inSpotlight
                      ? "voice-room-spotlight"
                      : "voice-room-filmstrip-tile"
                  }
                >
                  <VoiceTile
                    {...sourceProps(tile)}
                    stream={tile.stream}
                    label={tile.name}
                    screen={tile.kind === "live"}
                    live={tile.kind === "live"}
                    mirror={tile.mirror ?? false}
                    speaking={tile.speaking ?? false}
                    spotlight={inSpotlight}
                    pinned={pinned === tile.id}
                    onTogglePin={() => togglePin(tile.id)}
                    onSelect={
                      inSpotlight ? undefined : () => setPinned(tile.id)
                    }
                  />
                </div>
              );
            })}
        </div>
      ) : null}
      <ul
        className="voice-room-participants"
        aria-label="Teilnehmer im Sprachkanal"
      >
        {occupants.map(([id, flags]) => {
          const camera = cameras.get(id);
          return (
            <Participant
              key={id}
              id={id}
              member={members.get(id)}
              self={id === me}
              fallbackName={user?.name ?? "Du"}
              fallbackAvatar={id === me ? (user?.avatar_url ?? null) : null}
              muted={id === me && here ? voice.muted : flags.muted}
              deafened={id === me && here ? voice.deafened : flags.deafened}
              speaking={speaks(id, flags)}
              media={isLive(id) ? "Live" : camera ? "Kamera an" : null}
              camera={camera && camera !== spotlight ? camera : null}
              pinned={pinned}
              onTogglePin={togglePin}
              channelName={channelName}
              onProfile={openProfile}
            />
          );
        })}
        {Array.from({ length: freeSeats }, (_, seat) => (
          <li key={`seat-${seat}`} className="voice-room-seat" aria-hidden>
            <span className="voice-room-seat-circle" />
            <span>Platz frei</span>
          </li>
        ))}
      </ul>
      {!here && allowed && elsewhere ? (
        <p className="voice-room-muted voice-room-switch-note">
          Du wechselst aus {voice.channelName ?? "deinem Sprachkanal"}. Stumm
          und Taub bleiben erhalten.
        </p>
      ) : null}
      {!here ? (
        <div className="voice-room-actions">
          {allowed ? (
            <Button
              variant="primary"
              size="lg"
              icon={<SpeakerIcon size={20} />}
              onClick={onJoin}
              className="voice-room-join"
            >
              {elsewhere ? "Hierher wechseln" : "Dazukommen"}
            </Button>
          ) : (
            <p className="voice-room-muted voice-room-denied">
              Du hast in diesem Server kein Recht, Voice beizutreten.
            </p>
          )}
          <IconButton
            variant="secondary"
            label="Mikrofon testen"
            icon={<MicIcon size={18} />}
            onClick={() => setTestingMicrophone(roomKey)}
          />
          <QuickCallSettingsButton variant="secondary" />
        </div>
      ) : null}
      {profile?.serverId === server.id ? (
        <MemberProfileDialog
          server={server}
          memberId={profile.memberId}
          onClose={() => setProfile(null)}
        />
      ) : null}
      {testingMicrophone === roomKey ? (
        <MicrophoneTest
          key={`${server.id}/${channelId}`}
          onClose={() => setTestingMicrophone(null)}
        />
      ) : null}
    </section>
  );
}

/** One person in the room, once: their camera in place of the avatar while
 * it is on. "…", a long press or a right click opens their volume. */
function Participant({
  id,
  member,
  self,
  fallbackName,
  fallbackAvatar,
  muted,
  deafened,
  speaking,
  media,
  camera,
  pinned,
  onTogglePin,
  channelName,
  onProfile,
}: {
  id: string;
  member: Member | undefined;
  self: boolean;
  fallbackName: string;
  fallbackAvatar: string | null;
  muted: boolean;
  deafened: boolean;
  speaking: boolean;
  media: string | null;
  camera: StageTile | null;
  pinned: string | null;
  onTogglePin: (id: string) => void;
  channelName: string;
  onProfile: (memberId: string) => void;
}) {
  const [menu, setMenu] = useState(false);
  const menuAnchor = useRef<HTMLButtonElement>(null);
  const hasMenu = Boolean(member) && !self;
  const [press] = useState(() => createLongPress(() => setMenu(true)));
  // Someone who leaves mid-press opens nothing.
  useEffect(() => press.up, [press]);
  const name = member?.name ?? (self ? fallbackName : "Mitglied");
  const status = deafened
    ? "Taub"
    : muted
      ? "Stumm"
      : speaking
        ? "Spricht"
        : "Im Raum";
  const profile = () => {
    if (press.consumeClick()) return;
    if (member) onProfile(id);
  };
  return (
    <li
      className={[
        "voice-room-participant",
        speaking ? "is-speaking" : "",
        camera ? "has-video" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      onPointerDown={hasMenu ? (event) => press.down(event) : undefined}
      onPointerMove={hasMenu ? (event) => press.move(event) : undefined}
      onPointerUp={hasMenu ? press.up : undefined}
      onPointerCancel={hasMenu ? press.up : undefined}
      onTouchEnd={
        hasMenu
          ? (event) => {
              // The finger that opened the menu lifts over its backdrop: no
              // click may follow and close it again.
              if (press.consumeClick()) event.preventDefault();
            }
          : undefined
      }
      onContextMenu={(event) => {
        // A right click, or Android's long press: the volume menu.
        if (!hasMenu) return;
        event.preventDefault();
        setMenu(true);
      }}
    >
      {camera ? (
        <VoiceTile
          stream={camera.stream}
          label={camera.name}
          mirror={camera.mirror ?? false}
          speaking={camera.speaking ?? false}
          caption={false}
          pinned={pinned === camera.id}
          onTogglePin={() => onTogglePin(camera.id)}
        />
      ) : (
        <button
          type="button"
          className="voice-room-avatar"
          aria-label={`Profil von ${name}`}
          disabled={!member}
          onClick={profile}
        >
          <Avatar
            name={name}
            url={member?.avatar_url ?? fallbackAvatar}
            size="lg"
            className={speaking ? "is-speaking" : ""}
          />
          <span className="voice-room-connection" aria-hidden="true" />
        </button>
      )}
      <span className="voice-room-participant-text">
        <strong>
          <button
            type="button"
            className="voice-room-profile-name"
            disabled={!member}
            onClick={profile}
          >
            {name}
            {self ? " (du)" : ""}
          </button>
        </strong>
        <span className="voice-room-participant-status">
          {status}
          {media ? ` · ${media}` : ""}
        </span>
      </span>
      <span className="voice-room-participant-marks">
        {self ? null : <VolumeChip userId={id} />}
        {self ? null : <LocallyMutedIcon userId={id} />}
        <VoiceStateIcons
          inVoice
          muted={muted}
          deafened={deafened}
          channelName={channelName}
        />
      </span>
      {hasMenu ? (
        <>
          <IconButton
            ref={menuAnchor}
            size="xs"
            label={`Optionen für ${name}`}
            aria-haspopup="dialog"
            aria-expanded={menu}
            icon={<MoreIcon size={16} />}
            onClick={() => setMenu((open) => !open)}
            className="voice-room-participant-more"
          />
          <ParticipantMenu
            open={menu}
            onClose={() => setMenu(false)}
            anchor={menuAnchor}
            userId={id}
            name={name}
            onProfile={() => {
              setMenu(false);
              onProfile(id);
            }}
          />
        </>
      ) : null}
    </li>
  );
}

function ParticipantMenu({
  open,
  onClose,
  anchor,
  userId,
  name,
  onProfile,
}: {
  open: boolean;
  onClose: () => void;
  anchor: RefObject<HTMLButtonElement | null>;
  userId: string;
  name: string;
  onProfile: () => void;
}) {
  // A bottom sheet shows the name as its title already.
  const sheet = useMediaQuery(POPOVER_SHEET_QUERY);
  return (
    <Popover
      open={open}
      onClose={onClose}
      anchor={anchor}
      label={name}
      placement="bottom-end"
      className="voice-participant-menu"
    >
      {sheet ? null : (
        <strong className="voice-participant-name">{name}</strong>
      )}
      <UserVolumeControl userId={userId} name={name} />
      <Button
        size="sm"
        variant="ghost"
        className="voice-participant-profile"
        onClick={onProfile}
      >
        Profil ansehen
      </Button>
    </Popover>
  );
}

/** A volume other than 100 % stays in sight on the card. */
function VolumeChip({ userId }: { userId: string }) {
  const level = useUserVolumes((s) => userVolumeOf(userId, s.users));
  if (level.muted || level.volume === 1) return null;
  return (
    <span className="voice-room-volume-chip" title="Lautstärke nur für dich">
      <span className="sr-only">Lautstärke </span>
      {Math.round(level.volume * 100)} %
    </span>
  );
}
