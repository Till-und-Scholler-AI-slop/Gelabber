// The presence modes of #590 at the top of the account menu: Automatic,
// Away, Do not disturb, Invisible. A mode other than Automatic asks for how
// long first (1 h, 8 h, 24 h, until changed), right below its row.

import { useState } from "react";

import {
  MODE_DURATIONS,
  PRESENCE_MODES,
  modeAt,
  setPresenceMode,
  usePresenceMode,
  type ModeDuration,
  type PresenceMode,
} from "../presence/mode.ts";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { notifyError } from "./toasts.ts";

/** The dot each mode shows, as on your avatar. */
const DOT: Record<PresenceMode, string> = {
  auto: "o",
  idle: "i",
  dnd: "d",
  invisible: "v",
};

/** Catalog keys start with a letter. */
const DURATION_KEY: Record<ModeDuration, "h1" | "h8" | "h24" | "forever"> = {
  "1h": "h1",
  "8h": "h8",
  "24h": "h24",
  forever: "forever",
};

export function PresenceModeMenu({ onDone }: { onDone: () => void }) {
  const t = useT(shell);
  const current = usePresenceMode((state) => modeAt(state));
  const [asking, setAsking] = useState<PresenceMode | null>(null);

  const choose = (mode: PresenceMode, duration?: ModeDuration) => {
    setAsking(null);
    onDone();
    setPresenceMode(mode, duration).catch((error: unknown) => {
      notifyError(error, t("presenceMode.notSaved"));
    });
  };

  return (
    <div role="group" aria-label={t("presenceMode.label")}>
      {PRESENCE_MODES.map((mode) => {
        const open = asking === mode;
        const hint =
          mode === "dnd" || mode === "invisible"
            ? t(`presenceMode.hint.${mode}`)
            : null;
        return (
          <div key={mode} className="presence-mode">
            <button
              type="button"
              role="menuitemradio"
              aria-checked={current === mode}
              aria-expanded={mode === "auto" ? undefined : open}
              className="shell-menu-item"
              data-presence-mode={mode}
              onClick={() => {
                if (mode === "auto") choose("auto");
                else setAsking(open ? null : mode);
              }}
            >
              <span
                className={`presence-mode__dot gel-presence-${DOT[mode]}`}
                aria-hidden="true"
              />
              <span className="shell-menu-label">
                {t(`presenceMode.${mode}`)}
                {hint && open ? <small>{hint}</small> : null}
              </span>
            </button>
            {open ? (
              <div
                role="group"
                aria-label={t("presenceMode.for")}
                className="presence-mode__durations"
              >
                {MODE_DURATIONS.map((duration) => (
                  <button
                    key={duration}
                    type="button"
                    role="menuitem"
                    className="shell-menu-item"
                    data-duration={duration}
                    onClick={() => choose(mode, duration)}
                  >
                    {t(`presenceMode.duration.${DURATION_KEY[duration]}`)}
                  </button>
                ))}
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
