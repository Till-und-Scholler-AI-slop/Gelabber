// Voice channel body: join is a local state flip, ICE runs afterwards.
// Camera and screen tiles render local streams immediately; SFU publish
// is background work and never rides the chat socket.

import { useState } from "react";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { VoiceStateIcons } from "../components/VoiceStateIcons.tsx";
import { MemberProfileDialog } from "../components/MemberProfileDialog.tsx";
import { Avatar } from "../components/Avatar.tsx";
import { GearIcon, SpeakerIcon } from "../components/Icons.tsx";
import { useSession } from "../auth/session.ts";
import {
  joinVoice,
  useVoice,
  watchLive,
  toggleSourceWatch,
} from "./session.ts";
import { useMediaSettings } from "./settings.ts";
import { isDesktopApp } from "./native/bridge.ts";
import { EMPTY_OCCUPANCY, liveOf, useVoiceRoster } from "./roster.ts";
import { VoiceTile } from "./VoiceTile.tsx";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import "./room.css";

export function VoiceRoom({
  server,
  channelId,
  channelName,
}: {
  server: ServerDetail;
  channelId: string;
  channelName: string;
}) {
  const allowed = can(server, "join_voice");
  const voice = useVoice();
  const user = useSession((s) => s.user);
  const me = user?.id;
  const roster = useVoiceRoster(
    (s) => s.byServer[server.id] ?? EMPTY_OCCUPANCY,
  );
  const liveUser = useVoiceRoster((s) => liveOf(s.live, server.id, channelId));
  const openSettings = useMediaSettings((s) => s.openDialog);
  const here =
    voice.status === "joined" &&
    voice.serverId === server.id &&
    voice.channelId === channelId;
  const watching =
    voice.watching &&
    voice.watchServerId === server.id &&
    voice.watchChannelId === channelId;
  const liveStreamFor = (id: string) =>
    (here ? voice.remote[id]?.l : null) ??
    (watching && voice.watchPublisherId === id ? voice.watchStream : null);
  const roomKey = `${server.id}/${channelId}`;
  const [testingMicrophone, setTestingMicrophone] = useState<string | null>(
    null,
  );
  const [profile, setProfile] = useState<{
    serverId: string;
    memberId: string;
  } | null>(null);
  const [focus, setFocus] = useState<string | null>(null);
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

  const screens: { id: string; stream: MediaStream | null; name: string }[] =
    [];
  const cameras: {
    id: string;
    stream: MediaStream | null;
    name: string;
    mirror: boolean;
  }[] = [];
  let liveTile: {
    id: string;
    stream: MediaStream | null;
    name: string;
  } | null = null;
  for (const [id] of occupants) {
    const pubs = here ? (voice.participants[id]?.pubs ?? []) : [];
    const self = here && id === me;
    const name = self
      ? `${members.get(id)?.name ?? "Du"} (du)`
      : (members.get(id)?.name ?? "Mitglied");
    const cameraOn = self ? voice.camera : pubs.includes("v");
    const sharing = self ? voice.sharing : pubs.includes("s");
    const isLive = self ? voice.live : liveUser === id;
    if (isLive && !liveTile) {
      liveTile = {
        id: `${id}-l`,
        stream: self ? voice.localLive : liveStreamFor(id),
        name: `${name} — Live`,
      };
    }
    if (sharing) {
      screens.push({
        id: `${id}-s`,
        stream: self ? voice.localScreen : (voice.remote[id]?.s ?? null),
        name: `${name} — Bildschirm`,
      });
    }
    if (cameraOn) {
      cameras.push({
        id: `${id}-v`,
        stream: self ? voice.localCamera : (voice.remote[id]?.v ?? null),
        name,
        mirror: self,
      });
    }
  }

  if (!liveTile && liveUser) {
    const name =
      liveUser === me
        ? `${members.get(liveUser)?.name ?? "Du"} (du)`
        : (members.get(liveUser)?.name ?? "Mitglied");
    liveTile = {
      id: `${liveUser}-l`,
      stream:
        here && liveUser === me ? voice.localLive : liveStreamFor(liveUser),
      name: `${name} — Live`,
    };
  }

  const sourceProps = (tileId: string) => {
    const userId = tileId.slice(0, -2);
    const kind = tileId.endsWith("-s") ? ("s" as const) : ("l" as const);
    if (userId === me && here) {
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
                  : "Ton wird geteilt"
                : undefined,
      };
    }
    return here && voice.sourceWatchSupported
      ? {
          sourceWatch: {
            watching: voice.sourceSubscriptions[userId]?.[kind] === true,
            toggle: () => toggleSourceWatch(userId, kind),
          },
        }
      : {};
  };

  const showStage =
    Boolean(liveTile) || screens.length > 0 || cameras.length > 0;
  const liveOn = Boolean(liveUser) || (here && voice.live);
  const focusLive = liveTile && focus === liveTile.id;
  const focusScreen = screens.find((tile) => tile.id === focus);
  const focusCamera = cameras.find((tile) => tile.id === focus);
  const focused =
    focusLive && liveTile
      ? { kind: "live" as const, tile: liveTile }
      : focusScreen
        ? { kind: "screen" as const, tile: focusScreen }
        : focusCamera
          ? { kind: "camera" as const, tile: focusCamera }
          : null;

  const toggleFocus = (id: string) => {
    setFocus((current) => (current === id ? null : id));
  };

  return (
    <section className="voice-room" aria-label={channelName}>
      <header className="voice-room-heading">
        <SpeakerIcon size={28} />
        <div>
          <h2>
            {channelName}
            {liveOn ? (
              <span className="voice-room-live-badge">Live</span>
            ) : null}
          </h2>
          <p>{here ? "Schön, dass du da bist." : "Einfach dazukommen."}</p>
        </div>
      </header>
      {occupants.length === 0 ? (
        <p className="voice-room-empty">
          Niemand ist in diesem Kanal. Mach es dir gemütlich.
        </p>
      ) : null}
      <ul
        className="voice-room-participants"
        aria-label="Teilnehmer im Sprachkanal"
      >
        {occupants.map(([id, flags]) => {
          const member = members.get(id);
          const self = id === me;
          const name =
            member?.name ?? (self ? (user?.name ?? "Du") : "Mitglied");
          const pubs = here ? (voice.participants[id]?.pubs ?? []) : [];
          const muted = self && here ? voice.muted : flags.muted;
          const deafened = self && here ? voice.deafened : flags.deafened;
          const isLive = liveUser === id || (self && here && voice.live);
          const status = deafened ? "Taub" : muted ? "Stumm" : "Im Raum";
          const media = isLive
            ? "Live"
            : pubs.includes("s") || (self && here && voice.sharing)
              ? "Teilt den Bildschirm"
              : pubs.includes("v") || (self && here && voice.camera)
                ? "Kamera an"
                : null;
          return (
            <li key={id} className="voice-room-participant">
              <button
                type="button"
                className="voice-room-avatar"
                aria-label={`Profil von ${name}`}
                disabled={!member}
                onClick={() =>
                  setProfile({ serverId: server.id, memberId: id })
                }
              >
                <Avatar
                  name={name}
                  url={
                    member?.avatar_url ??
                    (self ? (user?.avatar_url ?? null) : null)
                  }
                  size="lg"
                />
                <span className="voice-room-connection" aria-hidden="true" />
              </button>
              <strong>
                <button
                  type="button"
                  className="voice-room-profile-name"
                  disabled={!member}
                  onClick={() =>
                    setProfile({ serverId: server.id, memberId: id })
                  }
                >
                  {name}
                  {self ? " (du)" : ""}
                </button>
              </strong>
              <span className="voice-room-participant-status">{status}</span>
              {media ? (
                <span className="voice-room-participant-media">{media}</span>
              ) : null}
              <VoiceStateIcons
                inVoice
                muted={muted}
                deafened={deafened}
                channelName={channelName}
              />
            </li>
          );
        })}
        {!here && user ? (
          <li className="voice-room-participant voice-room-disconnected">
            <div className="voice-room-avatar">
              <Avatar
                name={user.name ?? "Du"}
                url={user.avatar_url ?? null}
                size="lg"
              />
              <span className="voice-room-connection" aria-hidden="true" />
            </div>
            <strong>{user.name ?? "Du"} (du)</strong>
            <span className="voice-room-participant-status">
              Du bist noch nicht verbunden
            </span>
          </li>
        ) : null}
      </ul>
      {!here ? (
        <div className="voice-room-actions">
          {allowed ? (
            <button
              type="button"
              aria-label="Beitreten"
              onClick={onJoin}
              className="voice-room-button voice-room-join"
            >
              <SpeakerIcon size={20} />
              Dazukommen
            </button>
          ) : (
            <p className="voice-room-muted">
              Du hast in diesem Server kein Recht, Voice beizutreten.
            </p>
          )}
          <button
            type="button"
            onClick={() => setTestingMicrophone(roomKey)}
            className="voice-room-text-button"
          >
            Mikrofon testen
          </button>
          <button
            type="button"
            aria-label="Voice-Einstellungen"
            onClick={() => openSettings()}
            className="voice-room-text-button"
          >
            <GearIcon size={16} />
            Einstellungen
          </button>
        </div>
      ) : null}
      {!here && allowed && voice.status === "joined" ? (
        <p className="voice-room-muted voice-room-switch-note">
          Du wechselst aus {voice.channelName ?? "deinem Sprachkanal"}. Stumm
          und Taub bleiben erhalten.
        </p>
      ) : null}
      {!watching && !here && liveOn && allowed ? (
        <div className="voice-room-watch-actions">
          <button
            type="button"
            onClick={() =>
              watchLive({ serverId: server.id, channelId, channelName })
            }
            className="voice-room-button"
          >
            Zuschauen
          </button>
        </div>
      ) : null}
      {showStage ? (
        <div className="voice-room-stage" aria-label="Medien im Raum">
          <div className="voice-room-focus">
            <FocusChip
              active={!focused}
              onClick={() => setFocus(null)}
              label="Raster"
            />
            {liveTile ? (
              <FocusChip
                active={Boolean(focusLive)}
                onClick={() => setFocus(liveTile.id)}
                label="Live"
              />
            ) : null}
            {screens.length > 0 ? (
              <FocusChip
                active={Boolean(focusScreen)}
                onClick={() => setFocus(screens[0]?.id ?? null)}
                label="Bildschirm"
              />
            ) : null}
            {cameras.length > 0 ? (
              <FocusChip
                active={Boolean(focusCamera)}
                onClick={() => setFocus(cameras[0]?.id ?? null)}
                label="Kamera"
              />
            ) : null}
          </div>
          {focused ? (
            <VoiceTile
              key={focused.tile.id}
              {...(focused.kind === "camera"
                ? {}
                : sourceProps(focused.tile.id))}
              stream={focused.tile.stream}
              label={focused.tile.name}
              screen={focused.kind !== "camera"}
              live={focused.kind === "live"}
              mirror={
                focused.kind === "camera"
                  ? Boolean("mirror" in focused.tile && focused.tile.mirror)
                  : false
              }
              expanded
              onToggleExpand={() => toggleFocus(focused.tile.id)}
            />
          ) : (
            <>
              {liveTile ? (
                <VoiceTile
                  key={liveTile.id}
                  {...sourceProps(liveTile.id)}
                  stream={liveTile.stream}
                  label={liveTile.name}
                  screen
                  live
                  onToggleExpand={() => toggleFocus(liveTile.id)}
                />
              ) : null}
              {screens.map((tile) => (
                <VoiceTile
                  key={tile.id}
                  {...sourceProps(tile.id)}
                  stream={tile.stream}
                  label={tile.name}
                  screen
                  onToggleExpand={() => toggleFocus(tile.id)}
                />
              ))}
              {cameras.length > 0 ? (
                <div className="voice-room-cameras">
                  {cameras.map((tile) => (
                    <VoiceTile
                      key={tile.id}
                      stream={tile.stream}
                      label={tile.name}
                      mirror={tile.mirror}
                      onToggleExpand={() => toggleFocus(tile.id)}
                    />
                  ))}
                </div>
              ) : null}
            </>
          )}
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

function FocusChip({
  active,
  onClick,
  label,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      className="voice-room-focus-chip"
    >
      {label}
    </button>
  );
}
