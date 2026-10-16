import { useId, type CSSProperties } from "react";
import { MAX_PLAYBACK_VOLUME } from "../voice/settings.ts";
import { userVolumeOf, useUserVolumes } from "../voice/userVolume.ts";
import { SpeakerOffIcon } from "./Icons.tsx";

/** One participant's playback volume and local mute, for this client only. */
export function UserVolumeControl({
  userId,
  name,
}: {
  userId: string;
  name: string;
}) {
  const id = useId();
  const level = useUserVolumes((s) => userVolumeOf(userId, s.users));
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
        max={MAX_PLAYBACK_VOLUME * 100}
        step={1}
        value={Math.round(level.volume * 100)}
        style={rangeFill(level.volume)}
        aria-label={`Lautstärke von ${name}`}
        disabled={level.muted}
        onChange={(event) =>
          setVolume(userId, Number(event.target.value) / 100)
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
      <p className="user-volume__hint">Nur du hörst den Unterschied.</p>
    </section>
  );
}

/** A small marker for a participant this client has muted. */
export function LocallyMutedIcon({ userId }: { userId: string }) {
  const muted = useUserVolumes((s) => userVolumeOf(userId, s.users).muted);
  if (!muted) return null;
  return (
    <span className="user-volume-muted" title="Lokal stummgeschaltet">
      <SpeakerOffIcon size={13} />
      <span className="sr-only">Lokal stummgeschaltet</span>
    </span>
  );
}

/** Filled share of a 0–200 % `.lr-range` slider (WebKit has no progress
 * pseudo). */
function rangeFill(value: number): CSSProperties {
  return {
    "--lr-range-value": `${Math.round((value / MAX_PLAYBACK_VOLUME) * 100)}%`,
  } as CSSProperties;
}
