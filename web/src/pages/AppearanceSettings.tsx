import { Link } from "@tanstack/react-router";
import { useEffect } from "react";

import { SettingsGroup } from "../components/SettingsGroup.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import {
  useInterfacePreferences,
  type UiScale,
} from "../interface/preferences.ts";
import { changeTheme, syncThemes, useAccountThemes } from "../theme/account.ts";
import { BUILTIN_THEMES, builtinTheme } from "../theme/presets.ts";

const MODES = [
  { id: "light", label: "Hell", hint: "Gelabber Hell" },
  { id: "dark", label: "Dunkel", hint: "Gelabber Dunkel" },
  { id: "system", label: "System", hint: "Hell oder Dunkel wie dein Gerät" },
] as const;

const SCALES: { value: UiScale; label: string }[] = [
  { value: "compact", label: "Kompakt" },
  { value: "standard", label: "Standard" },
  { value: "large", label: "Groß" },
];

/** A small picture of the theme on its card: page, panel, text and accent. */
function ModeSwatch({ id }: { id: "light" | "dark" | "system" }) {
  if (id === "system")
    return (
      <span className="settings-mode-swatch settings-mode-swatch--split">
        <ModeSwatch id="light" />
        <ModeSwatch id="dark" />
      </span>
    );
  const { colors } = builtinTheme(id);
  return (
    <span
      className="settings-mode-swatch"
      style={{ background: colors.background, borderColor: colors.border }}
    >
      <i style={{ background: colors.panel }} />
      <b style={{ background: colors.text }} />
      <em style={{ background: colors.accent }} />
    </span>
  );
}

/** Settings area "Darstellung": light or dark, text size, room density. */
export function AppearanceSettings() {
  const preferences = useInterfacePreferences();
  const themes = useAccountThemes();
  const status = useInlineStatus();
  const active = themes.doc.active;
  // Not disabled while a choice saves: a disabled radio would drop the
  // keyboard focus that just picked it.
  const unavailable =
    themes.status === "loading" ||
    themes.status === "error" ||
    themes.status === "conflict";
  const saving = themes.status === "saving" || !!themes.pending;
  const otherTheme = MODES.some((mode) => mode.id === active)
    ? null
    : (themes.doc.customThemes.find((theme) => theme.id === active) ??
      BUILTIN_THEMES.find((theme) => theme.id === active));

  useEffect(() => {
    void syncThemes();
  }, []);

  return (
    <>
      <SettingsGroup title="Farbschema" scope="account">
        <div className="settings-mode-options">
          {MODES.map((mode) => (
            <label key={mode.id} className="settings-mode-option">
              <input
                type="radio"
                name="theme-mode"
                className="ui-check"
                checked={active === mode.id}
                disabled={unavailable}
                onChange={async () => {
                  if (saving) return;
                  status.show(
                    (await changeTheme({ kind: "apply", id: mode.id }))
                      ? { tone: "success", message: "Gespeichert." }
                      : {
                          tone: "error",
                          message: "Nicht gespeichert – erneut versuchen",
                        },
                  );
                }}
              />
              <ModeSwatch id={mode.id} />
              <span className="ui-choice-text">
                {mode.label}
                <small>{mode.hint}</small>
              </span>
            </label>
          ))}
        </div>
        {unavailable && themes.error ? (
          <p className="ui-field-error">{themes.error}</p>
        ) : null}
        <p className="settings-hint">
          {otherTheme ? `Gerade aktiv: ${otherTheme.name}. ` : null}
          <Link
            to="/settings/$area"
            params={{ area: "themes" }}
            className="settings-link"
          >
            Weitere Themes
          </Link>
        </p>
        <InlineStatus status={status.status} />
      </SettingsGroup>

      <SettingsGroup title="Dein Wohnzimmer" scope="device">
        <div className="settings-row settings-row--stacked">
          <span className="ui-choice-text">
            Textgröße
            <small>
              Vergrößert die ganze Oberfläche. Kompakt passt am meisten auf den
              Bildschirm.
            </small>
          </span>
          <ChipGroup
            label="Textgröße"
            options={SCALES}
            value={preferences.uiScale}
            onChange={(uiScale) => preferences.patch({ uiScale })}
          />
        </div>
        <Switch
          className="settings-row"
          label="Kompakte Raumansicht"
          description="Kleinere Avatare und weniger Abstand."
          checked={preferences.compactRooms}
          onChange={(event) =>
            preferences.patch({ compactRooms: event.target.checked })
          }
        />
        <Switch
          className="settings-row"
          label="Bewegung reduzieren"
          description="Berücksichtigt auch die Einstellung deines Geräts."
          checked={preferences.reducedMotion}
          onChange={(event) =>
            preferences.patch({ reducedMotion: event.target.checked })
          }
        />
      </SettingsGroup>
    </>
  );
}
