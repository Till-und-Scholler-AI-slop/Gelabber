// Voice channel body: join is a local state flip, ICE runs afterwards.
// Camera and screen tiles render local streams immediately; SFU publish
// is background work and never rides the chat socket.
//
// On the channel's page, every picture in the call gets a tile of its own
// on the stage, and the tiles together fill it; the people without a
// camera line up under it (Raster). In Fokus one picture fills the stage
// over a row of the others and the people; it is the view while someone
// else is live, unless the user picked one in this call. The overview keeps its Lives in a spotlight and
// a filmstrip, and a camera in its owner's place. A Live is watched in the
// room: outside it, the room only says who is live.

import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
} from "react";

import { can } from "../servers/permissions.ts";
import type { ServerDetail } from "../servers/types.ts";
import { MemberProfileDialog } from "../components/MemberProfileDialog.tsx";
import {
  FocusIcon,
  GridIcon,
  MicIcon,
  SpeakerIcon,
} from "../components/Icons.tsx";
import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import { setMemberPanelVideo } from "../components/memberPanelState.ts";
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
import { Participant } from "./Participant.tsx";
import { MicrophoneTest } from "./MicrophoneTest.tsx";
import { QuickCallSettingsButton } from "./QuickCallSettings.tsx";
import { roomSummary, stageFacts } from "./roomSummary.ts";
import {
  arrangeStage,
  focusOf,
  focusRects,
  rasterOrder,
  rasterRects,
  type Rect,
  type StageKind,
} from "./stage.ts";
import {
  choicesFor,
  chooseView,
  pinPicture,
  stageViewFor,
  useStageChoices,
  type StageView,
} from "./stageView.ts";
import "./room.css";

type StageTile = {
  id: string;
  kind: StageKind;
  /** The track of a Live: a screen. */
  source?: "s";
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
  overview = false,
}: {
  server: ServerDetail;
  channelId: string;
  channelName: string;
  /** The overview's room: its name heads it, with "Verbunden" while in the
   * call, and the people are all it shows; no line about them, no free
   * seats. The channel's page names the room itself. */
  overview?: boolean;
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
  const choices = useStageChoices((state) => choicesFor(state, roomKey));
  const pinned = choices.pinned;
  // A phone keeps its column of pictures; Fokus and its row are for wider
  // screens.
  const narrow = useMediaQuery("(max-width: 640px)");
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
    if (screen) {
      const stream = self ? voice.localScreen : voice.remote[id]?.s;
      shares.push({
        id: `${id}-s`,
        kind: "live",
        source: "s",
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
    (id === me && here && voice.sharing);

  const sourceProps = (tile: StageTile) => {
    if (!tile.source) return {};
    const kind = tile.source;
    if (tile.userId === me && here) {
      const status = voice.sourceAudio.s;
      // The reason stands next to the call controls. A picker withholds
      // sound in a browser only; the desktop app captures it by itself.
      const silent = voice.sourceAudioNote.s?.silent;
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
    pinPicture(roomKey, pinned === id ? null : id);
  const stageTiles = spotlight ? [spotlight, ...filmstrip] : [];
  // The channel's page: a stage of its own while any picture runs.
  const pictures = overview ? [] : [...shares, ...cameras.values()];
  const staged = pictures.length > 0;
  useEffect(() => {
    if (overview) return;
    setMemberPanelVideo(staged);
    return () => setMemberPanelVideo(false);
  }, [overview, staged]);
  const [stageRef, stageSize] = useBoxSize<HTMLUListElement>(staged);
  const view: StageView = narrow
    ? "raster"
    : stageViewFor(
        choices.view,
        shares.some((tile) => tile.userId !== me),
      );
  const focus =
    view === "fokus"
      ? focusOf(shares, [...cameras.values()], pinned, me)
      : null;
  const ordered = rasterOrder(shares, [...cameras.values()], pinned);
  // Fokus: the picture in front, the others and then the people without a
  // camera in the row under it.
  const strip = ordered.filter((tile) => tile !== focus);
  const stripPeople = focus ? occupants.filter(([id]) => !cameras.has(id)) : [];
  const layout = !stageSize
    ? null
    : focus
      ? focusRects(
          strip.length + stripPeople.length,
          stageSize.width,
          stageSize.height,
        )
      : {
          focus: null,
          strip: rasterRects(ordered.length, stageSize.width, stageSize.height),
        };
  /** A picture's or person's place on the stage, for the stylesheet. */
  const at = (rect: Rect | null | undefined): CSSProperties | undefined =>
    rect
      ? ({
          "--stage-x": `${rect.x}px`,
          "--stage-y": `${rect.y}px`,
          "--stage-w": `${rect.width}px`,
          "--stage-h": `${rect.height}px`,
        } as CSSProperties)
      : undefined;
  const placed = (tile: StageTile) =>
    at(
      tile === focus
        ? layout?.focus
        : layout?.strip[(focus ? strip : ordered).indexOf(tile)],
    );
  /** Where the stage puts a picture, for its classes. */
  const place = (tile: StageTile) =>
    tile === focus ? "is-focus" : focus ? "is-strip" : "";
  const bringForward = (tile: StageTile) =>
    focus && tile !== focus ? () => pinPicture(roomKey, tile.id) : undefined;
  const liveOn = occupants.some(([id]) => isLive(id));
  const others = occupants
    .filter(([id]) => id !== me)
    .map(([id]) => members.get(id)?.name ?? "Jemand");
  const othersLive = occupants
    .filter(([id]) => id !== me && isLive(id))
    .map(([id]) => members.get(id)?.name ?? "Jemand");
  const elsewhere = voice.status === "joined" && !here;
  const freeSeats =
    here || !allowed || overview
      ? 0
      : occupants.length === 0
        ? FREE_SEATS_EMPTY
        : 1;

  const person = (id: string, flags: { muted: boolean; deafened: boolean }) => {
    const camera = cameras.get(id);
    return {
      id,
      member: members.get(id),
      self: id === me,
      fallbackName: user?.name ?? "Du",
      fallbackAvatar: id === me ? (user?.avatar_url ?? null) : null,
      muted: id === me && here ? voice.muted : flags.muted,
      deafened: id === me && here ? voice.deafened : flags.deafened,
      speaking: speaks(id, flags),
      media: isLive(id) ? "Live" : camera ? "Kamera an" : null,
      pinned,
      onTogglePin: togglePin,
      channelName,
      onProfile: openProfile,
    };
  };
  const flagsOf = new Map(occupants);
  const noFlags = { muted: false, deafened: false };

  return (
    <section
      className="voice-room"
      aria-label={channelName}
      data-joined={here || undefined}
      data-page={!overview || undefined}
      data-stage={staged || undefined}
    >
      {overview ? (
        <div className="voice-room-heading">
          <SpeakerIcon size={18} />
          <h2 className="voice-room-name">{channelName}</h2>
          {here ? (
            <span className="voice-room-connected">Verbunden</span>
          ) : null}
          {liveOn ? <span className="voice-room-live-badge">Live</span> : null}
        </div>
      ) : staged ? (
        <div className="voice-stage-bar">
          <p className="voice-stage-facts">
            {stageFacts({
              people: occupants.length,
              cameras: cameras.size,
              live: othersLive,
              selfLive: here && voice.sharing,
            })}
          </p>
          {narrow ? null : (
            <ChipGroup
              label="Ansicht"
              className="voice-stage-views"
              value={view}
              onChange={(next) => chooseView(roomKey, next)}
              options={[
                {
                  value: "raster",
                  label: (
                    <>
                      <GridIcon size={15} />
                      Raster
                    </>
                  ),
                },
                {
                  value: "fokus",
                  label: (
                    <>
                      <FocusIcon size={15} />
                      Fokus
                    </>
                  ),
                },
              ]}
            />
          )}
        </div>
      ) : (
        <p className="voice-room-summary">
          {roomSummary(others, { here, live: othersLive })}
        </p>
      )}
      {staged ? (
        <ul
          ref={stageRef}
          className="voice-stage"
          aria-label="Bühne"
          data-view={view}
          data-measured={layout ? "" : undefined}
        >
          {/* One element per picture, in a fixed order: a place on the
              stage is a position, so a pin or another view never rebuilds
              a video. */}
          {pictures.map((tile) =>
            tile.kind === "live" ? (
              <li
                key={tile.id}
                className={`voice-stage-item ${place(tile)}`.trim()}
                style={placed(tile)}
              >
                <VoiceTile
                  {...sourceProps(tile)}
                  stream={tile.stream}
                  label={tile.name}
                  screen
                  live
                  spotlight={tile === focus}
                  pinned={pinned === tile.id}
                  onTogglePin={() => togglePin(tile.id)}
                  onSelect={bringForward(tile)}
                />
              </li>
            ) : (
              <Participant
                key={tile.id}
                {...person(tile.userId, flagsOf.get(tile.userId) ?? noFlags)}
                camera={tile}
                tile
                spotlight={tile === focus}
                onSelect={bringForward(tile)}
                className={`voice-stage-item ${place(tile)}`.trim()}
                style={placed(tile)}
              />
            ),
          )}
          {stripPeople.map(([id, flags], index) => (
            <Participant
              key={id}
              {...person(id, flags)}
              camera={null}
              className="voice-stage-item is-strip"
              style={at(layout?.strip[strip.length + index])}
            />
          ))}
        </ul>
      ) : null}
      {overview && stageTiles.length > 0 ? (
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
                      inSpotlight
                        ? undefined
                        : () => pinPicture(roomKey, tile.id)
                    }
                  />
                </div>
              );
            })}
        </div>
      ) : null}
      {staged ? (
        !focus && occupants.some(([id]) => !cameras.has(id)) ? (
          <ul
            className="voice-room-participants voice-stage-people"
            aria-label="Ohne Kamera"
          >
            {occupants
              .filter(([id]) => !cameras.has(id))
              .map(([id, flags]) => (
                <Participant key={id} {...person(id, flags)} camera={null} />
              ))}
          </ul>
        ) : null
      ) : occupants.length > 0 || freeSeats > 0 ? (
        <ul
          className="voice-room-participants"
          aria-label="Teilnehmer im Sprachkanal"
        >
          {occupants.map(([id, flags]) => {
            const camera = cameras.get(id);
            return (
              <Participant
                key={id}
                {...person(id, flags)}
                camera={camera && camera !== spotlight ? camera : null}
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
      ) : null}
      {!here && allowed && elsewhere ? (
        <p className="voice-room-muted voice-room-switch-note">
          Du verlässt dabei {voice.channelName ?? "deinen Sprachkanal"}.
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
              {elsewhere ? "Hierher wechseln" : "Beitreten"}
            </Button>
          ) : (
            <p className="voice-room-muted voice-room-denied">
              Keine Berechtigung für Sprachkanäle.
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

/** The element's size in CSS pixels while `active`, followed as it
 * changes; null before the first measure. */
function useBoxSize<T extends HTMLElement>(
  active: boolean,
): [RefObject<T | null>, { width: number; height: number } | null] {
  const ref = useRef<T>(null);
  const [size, setSize] = useState<{ width: number; height: number } | null>(
    null,
  );
  useLayoutEffect(() => {
    const element = ref.current;
    if (!active || !element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() =>
      setSize((current) => {
        // Client sizes are CSS pixels, whatever the Textgröße zooms.
        const width = element.clientWidth;
        const height = element.clientHeight;
        return current?.width === width && current.height === height
          ? current
          : { width, height };
      }),
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [active]);
  return [ref, active ? size : null];
}
