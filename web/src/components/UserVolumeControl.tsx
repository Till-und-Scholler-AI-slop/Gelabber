import { useId, type CSSProperties } from "react";
import {
  PLAYBACK_SLIDER_MAX,
  playbackSliderPosition,
  playbackSliderVolume,
} from "../voice/settings.ts";
import {
  userVolumeOf,
  useUserVolumes,
  type UserVolume,
} from "../voice/userVolume.ts";
import { SpeakerOffIcon } from "./Icons.tsx";

/** One participant's playback volume and local mute, for this client only. */
export function UserVolumeControl({
  userId,
  name,
}: {
  userId: string;
  name: string;
}) {
  const level = useUserVolumes((s) => userVolumeOf(userId, s.users));
  return <UserVolumeView userId={userId} name={name} level={level} />;
}

/** The control for a given level; the store's actions change it. */
export function UserVolumeView({
  userId,
  name,
  level,
}: {
  userId: string;
  name: string;
  level: UserVolume;
}) {
  const id = useId();
  const { setVolume, setMuted, reset } = useUserVolumes.getState();
  const changed = level.volume !== 1 || level.muted;
  return (
    <section className="user-volume" aria-label={`Lautstärke von ${name}`}>
      <div className="user-volume__row">
        <label htmlFor={`${id}-volume`}>Lautstärke</label>
        <output htmlFor={`${id}-volume`}>
          {Math.round(level.volume * 100)} %
        </output>
      </div>
      <input
        id={`${id}-volume`}
        type="range"
        min={0}
        max={PLAYBACK_SLIDER_MAX}
        step={1}
        value={playbackSliderPosition(level.volume)}
        style={rangeFill(level.volume)}
        aria-valuetext={`${Math.round(level.volume * 100)} %`}
        disabled={level.muted}
        onChange={(event) =>
          setVolume(userId, playbackSliderVolume(Number(event.target.value)))
        }
        className="lr-range"
      />
      <div className="user-volume__row">
        <label className="user-volume__mute">
          <input
            type="checkbox"
            checked={level.muted}
            onChange={(event) => setMuted(userId, event.target.checked)}
          />
          Lokal stummschalten
        </label>
        <button
          type="button"
          className="user-volume__reset"
          disabled={!changed}
          onClick={() => reset(userId)}
        >
          Zurücksetzen
        </button>
      </div>
      <p className="user-volume__hint">
        Gilt für die Stimme, nicht für Stream-Ton. Nur du hörst den Unterschied.
      </p>
    </section>
  );
}

/** A small marker for a participant this client has muted. */
export function LocallyMutedIcon({ userId }: { userId: string }) {
  const muted = useUserVolumes((s) => userVolumeOf(userId, s.users).muted);
  return muted ? <LocallyMutedMark /> : null;
}

export function LocallyMutedMark() {
  return (
    <span className="user-volume-muted" title="Lokal stummgeschaltet">
      <SpeakerOffIcon size={13} />
      <span className="sr-only">Lokal stummgeschaltet</span>
    </span>
  );
}

/** Filled share of a `.lr-range` playback slider (WebKit has no progress
 * pseudo). */
function rangeFill(value: number): CSSProperties {
  return {
    "--lr-range-value": `${(playbackSliderPosition(value) * 100) / PLAYBACK_SLIDER_MAX}%`,
  } as CSSProperties;
}
