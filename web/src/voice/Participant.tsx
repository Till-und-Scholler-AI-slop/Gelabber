// One person in a voice room: a card with their avatar, or their camera in
// its place; on the channel's stage a camera tile or a chip in the row of
// people without one. "…", a long press or a right click opens their
// volume and profile.

import {
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from "react";

import type { Member } from "../servers/types.ts";
import { VoiceStateIcons } from "../components/VoiceStateIcons.tsx";
import { Avatar } from "../components/Avatar.tsx";
import { MoreIcon } from "../components/Icons.tsx";
import { Button } from "../components/ui/Button.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { Popover } from "../components/ui/Popover.tsx";
import { POPOVER_SHEET_QUERY } from "../components/ui/popoverPosition.ts";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import {
  LocallyMutedIcon,
  UserVolumeControl,
} from "../components/UserVolumeControl.tsx";
import { userVolumeOf, useUserVolumes } from "./userVolume.ts";
import { createLongPress } from "./stage.ts";
import { VoiceTile } from "./VoiceTile.tsx";
import "./room.css";

/** A camera as the room shows it. */
export type CameraTile = {
  id: string;
  stream: MediaStream | null;
  name: string;
  mirror?: boolean;
  speaking?: boolean;
};

/** One person in the room, once: their camera in place of the avatar while
 * it is on. "…", a long press or a right click opens their volume.
 *
 * As a `tile` on the channel's stage the camera fills the item, the name
 * and marks on it in one label; `className` and `style` place it there. */
export function Participant({
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
  tile = false,
  className,
  style,
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
  camera: CameraTile | null;
  pinned: string | null;
  onTogglePin: (id: string) => void;
  channelName: string;
  onProfile: (memberId: string) => void;
  tile?: boolean;
  className?: string;
  style?: CSSProperties;
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
  const marks = (
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
  );
  return (
    <li
      className={[
        "voice-room-participant",
        speaking ? "is-speaking" : "",
        camera ? "has-video" : "",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
      style={style}
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
        {tile ? marks : null}
        <strong>
          <button
            type="button"
            className="voice-room-profile-name"
            disabled={!member}
            onClick={profile}
          >
            {name}
            {self ? <span className="voice-room-self"> (du)</span> : null}
          </button>
        </strong>
        {/* `data-media`: the overview shows this line only for a camera or
            a Live; the state is the corner's icons and the speaking ring.
            A tile and a chip say it to a screen reader only. */}
        <span
          className="voice-room-participant-status"
          data-media={media ? "" : undefined}
        >
          {status}
          {media ? ` · ${media}` : ""}
        </span>
      </span>
      {tile ? null : marks}
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
    <span className="voice-room-volume-chip" title="Lokale Lautstärke">
      <span className="sr-only">Lautstärke </span>
      {Math.round(level.volume * 100)} %
    </span>
  );
}
