// The areas of `/settings/$area`. Each one is its own URL, so reloads, the
// back button and links from elsewhere (the call dialog, a profile card)
// reach it directly.

import type { SettingsScope } from "../components/settingsScope.ts";
import { NAVIGATION_DRAWER_MAX_WIDTH } from "../components/WorkspaceNavigation.tsx";

export const SETTINGS_GROUPS = [
  { id: "konto", label: "Konto" },
  { id: "app", label: "App" },
] as const;

export type SettingsGroup = (typeof SETTINGS_GROUPS)[number]["id"];

export const SETTINGS_AREAS = [
  {
    id: "profil",
    scope: "account",
    group: "konto",
    label: "Profil",
    description: "Dein Name und dein Bild, in Chats und Sprachräumen.",
  },
  {
    id: "profilkarte",
    scope: "account",
    group: "konto",
    label: "Profilkarte",
    description:
      "So sehen dich andere, wenn sie auf dich tippen – im Browser, in der App und auf dem Handy.",
  },
  {
    id: "konto",
    scope: "account",
    group: "konto",
    label: "Sicherheit",
    description: "Dein Passwort und die Geräte, auf denen du angemeldet bist.",
  },
  {
    id: "darstellung",
    group: "app",
    label: "Darstellung",
    description: "Mach Gelabber zu deinem Platz.",
  },
  {
    id: "themes",
    scope: "account",
    group: "app",
    label: "Themes",
    description: "Farben und Designs, die zu dir passen.",
  },
  {
    id: "audio",
    scope: "device",
    group: "app",
    label: "Audio",
    description: "Dein Mikrofon, deine Lautsprecher und die Lautstärke.",
  },
  {
    id: "video",
    scope: "device",
    group: "app",
    label: "Video",
    description: "Kamera, Bildschirmfreigabe und Liveübertragungen.",
  },
  {
    id: "benachrichtigungen",
    group: "app",
    label: "Benachrichtigungen",
    description: "Entscheide, wie neue Nachrichten dich erreichen.",
  },
  // Last: installing is done once, the rest is what people come back for.
  {
    id: "app",
    group: "app",
    label: "App",
    description: "Gelabber wie eine eigene App öffnen.",
  },
] as const satisfies readonly {
  id: string;
  group: SettingsGroup;
  label: string;
  description: string;
  /** Set when every setting of the area is kept in the same place; mixed
   * areas tag each group instead. */
  scope?: SettingsScope;
}[];

export type SettingsArea = (typeof SETTINGS_AREAS)[number]["id"];
export type SettingsAreaInfo = (typeof SETTINGS_AREAS)[number];

/** Where `/settings` and the gear lead. */
export const DEFAULT_SETTINGS_AREA: SettingsArea = "darstellung";

export function isSettingsArea(value: unknown): value is SettingsArea {
  return SETTINGS_AREAS.some((area) => area.id === value);
}

export function settingsArea(id: SettingsArea): SettingsAreaInfo {
  return SETTINGS_AREAS.find((area) => area.id === id)!;
}

/**
 * The areas listed in the navigation. "App" is only offered where Gelabber
 * can still be installed, or while it is the open area.
 */
export function visibleSettingsAreas(
  installable: boolean,
  current?: SettingsArea,
): SettingsAreaInfo[] {
  return SETTINGS_AREAS.filter(
    (area) => area.id !== "app" || installable || current === "app",
  );
}

/**
 * Up to this width the workspace sidebar is a closed drawer, so `/settings`
 * is a list of the areas and each area a screen of its own.
 */
export const SETTINGS_LIST_QUERY = `(max-width: ${NAVIGATION_DRAWER_MAX_WIDTH}px)`;

export function settingsListFirst(): boolean {
  return (
    typeof window !== "undefined" &&
    window.matchMedia(SETTINGS_LIST_QUERY).matches
  );
}

declare module "@tanstack/history" {
  interface HistoryState {
    /** Set by the phone list of settings areas, so its back button can
     * return to the list instead of stacking it again. */
    settingsList?: boolean;
  }
}
