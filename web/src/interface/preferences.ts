import { create } from "zustand";

const STORAGE_KEY = "gelabber.interface";

/** Textgröße: compact is the owner's default density; the others enlarge it. */
export const UI_SCALES = {
  compact: 1,
  standard: 1.125,
  large: 1.25,
} as const;
export type UiScale = keyof typeof UI_SCALES;

type Preferences = {
  compactRooms: boolean;
  reducedMotion: boolean;
  uiScale: UiScale;
};
const defaults: Preferences = {
  compactRooms: false,
  reducedMotion: false,
  uiScale: "compact",
};

export function isUiScale(value: unknown): value is UiScale {
  return typeof value === "string" && Object.hasOwn(UI_SCALES, value);
}

export function parsePreferences(saved: unknown): Preferences {
  if (!saved || typeof saved !== "object") return defaults;
  return {
    compactRooms: "compactRooms" in saved && saved.compactRooms === true,
    reducedMotion: "reducedMotion" in saved && saved.reducedMotion === true,
    uiScale:
      "uiScale" in saved && isUiScale(saved.uiScale)
        ? saved.uiScale
        : defaults.uiScale,
  };
}

function load(): Preferences {
  try {
    return parsePreferences(
      JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null"),
    );
  } catch {
    return defaults;
  }
}

export const useInterfacePreferences = create<
  Preferences & { patch: (next: Partial<Preferences>) => void }
>((set) => ({
  ...load(),
  patch: (next) =>
    set((current) => {
      const value = {
        compactRooms: next.compactRooms ?? current.compactRooms,
        reducedMotion: next.reducedMotion ?? current.reducedMotion,
        uiScale: next.uiScale ?? current.uiScale,
      };
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
      } catch {
        /* Keep the controls usable when storage is disabled. */
      }
      return value;
    }),
}));
