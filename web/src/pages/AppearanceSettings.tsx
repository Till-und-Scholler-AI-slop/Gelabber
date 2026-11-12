import { Link } from "@tanstack/react-router";
import { useEffect } from "react";

import { SettingsGroup } from "../components/SettingsGroup.tsx";
import { Button } from "../components/ui/Button.tsx";
import { ChipGroup } from "../components/ui/ChipGroup.tsx";
import { InlineStatus } from "../components/ui/InlineStatus.tsx";
import { RadioCard } from "../components/ui/RadioCard.tsx";
import { Switch } from "../components/ui/Switch.tsx";
import { useInlineStatus } from "../components/ui/useInlineStatus.ts";
import { changeAccountLocale } from "../i18n/account.ts";
import { translate, useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import { language } from "../i18n/catalogs/language.ts";
import { settings } from "../i18n/catalogs/settings.ts";
import { LOCALES, useCurrentLocale } from "../i18n/locale.ts";
import {
  useInterfacePreferences,
  type UiScale,
} from "../interface/preferences.ts";
import {
  changeTheme,
  retryThemes,
  syncThemes,
  useAccountThemes,
} from "../theme/account.ts";
import { BUILTIN_THEMES, builtinTheme } from "../theme/presets.ts";

// Under Hell and Dunkel, the theme's name, which stays as it is.
const MODES = ["light", "dark", "system"] as const;

const SCALES: readonly UiScale[] = ["compact", "standard", "large"];

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

/**
 * The account's language. Each option is named in its own language, with
 * its name in the interface's language below when that differs.
 */
function LanguageSettings() {
  const t = useT(language);
  const locale = useCurrentLocale();
  const status = useInlineStatus();
  return (
    <SettingsGroup title={t("section.title")} scope="account">
      <div className="settings-language-options">
        {LOCALES.map((option) => {
          const label = t(`option.${option}`);
          const name = t(`name.${option}`);
          // Not disabled while a choice saves: a disabled radio would drop
          // the keyboard focus that just picked it.
          return (
            <RadioCard
              key={option}
              name="ui-language"
              label={label}
              description={name === label ? undefined : name}
              checked={locale === option}
              onChange={async () => {
                const saved = await changeAccountLocale(option);
                status.show({
                  tone: saved ? "success" : "error",
                  message: translate(common, saved ? "saved" : "notSaved"),
                });
              }}
            />
          );
        })}
      </div>
      <p className="settings-hint">{t("hint")}</p>
      <InlineStatus status={status.status} />
    </SettingsGroup>
  );
}

/** Settings area "Darstellung": light or dark, text size, room density. */
export function AppearanceSettings() {
  const t = useT(settings);
  const tc = useT(common);
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
  const otherTheme = MODES.some((mode) => mode === active)
    ? null
    : (themes.doc.customThemes.find((theme) => theme.id === active) ??
      BUILTIN_THEMES.find((theme) => theme.id === active));

  useEffect(() => {
    void syncThemes();
  }, []);

  return (
    <>
      <LanguageSettings />

      <SettingsGroup title={t("appearance.colorScheme")} scope="account">
        <div className="settings-mode-options">
          {MODES.map((mode) => (
            <label key={mode} className="settings-mode-option">
              <input
                type="radio"
                name="theme-mode"
                className="ui-check"
                checked={active === mode}
                disabled={unavailable}
                onChange={async () => {
                  if (saving) return;
                  status.show(
                    (await changeTheme({ kind: "apply", id: mode }))
                      ? { tone: "success", message: translate(common, "saved") }
                      : {
                          tone: "error",
                          message: translate(common, "notSaved"),
                        },
                  );
                }}
              />
              <ModeSwatch id={mode} />
              <span className="ui-choice-text">
                {t(`appearance.mode.${mode}`)}
                <small>
                  {mode === "system"
                    ? t("appearance.mode.systemHint")
                    : builtinTheme(mode).name}
                </small>
              </span>
            </label>
          ))}
        </div>
        {unavailable && themes.error ? (
          <>
            <p className="ui-field-error">{themes.error}</p>
            <Button size="sm" onClick={() => void retryThemes()}>
              {themes.pending ? t("appearance.retryApply") : tc("tryAgain")}
            </Button>
          </>
        ) : null}
        <p className="settings-hint">
          {otherTheme
            ? `${t("appearance.activeTheme", { name: otherTheme.name })} `
            : null}
          <Link
            to="/settings/$area"
            params={{ area: "themes" }}
            className="settings-link"
          >
            {t("appearance.moreThemes")}
          </Link>
        </p>
        <InlineStatus status={status.status} />
      </SettingsGroup>

      <SettingsGroup title={t("appearance.view")} scope="device">
        <div className="settings-row settings-row--stacked">
          <span className="ui-choice-text">
            {t("appearance.textSize")}
            <small>{t("appearance.textSizeHint")}</small>
          </span>
          <ChipGroup
            label={t("appearance.textSize")}
            options={SCALES.map((value) => ({
              value,
              label: t(`appearance.scale.${value}`),
            }))}
            value={preferences.uiScale}
            onChange={(uiScale) => preferences.patch({ uiScale })}
          />
        </div>
        <Switch
          className="settings-row"
          label={t("appearance.compactRooms")}
          description={t("appearance.compactRoomsHint")}
          checked={preferences.compactRooms}
          onChange={(event) =>
            preferences.patch({ compactRooms: event.target.checked })
          }
        />
        <Switch
          className="settings-row"
          label={t("appearance.reducedMotion")}
          description={t("appearance.reducedMotionHint")}
          checked={preferences.reducedMotion}
          onChange={(event) =>
            preferences.patch({ reducedMotion: event.target.checked })
          }
        />
      </SettingsGroup>
    </>
  );
}
