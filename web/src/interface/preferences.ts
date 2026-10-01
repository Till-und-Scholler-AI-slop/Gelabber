import { create } from "zustand";

const STORAGE_KEY = "gelabber.interface";
type Preferences = { compactRooms: boolean; reducedMotion: boolean };
const defaults: Preferences = { compactRooms: false, reducedMotion: false };
function load(): Preferences {
  try {
    const saved: unknown = JSON.parse(
      localStorage.getItem(STORAGE_KEY) ?? "null",
    );
    if (!saved || typeof saved !== "object") return defaults;
    return {
      compactRooms: "compactRooms" in saved && saved.compactRooms === true,
      reducedMotion: "reducedMotion" in saved && saved.reducedMotion === true,
    };
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
      };
      try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(value));
      } catch {
        /* Keep the controls usable when storage is disabled. */
      }
      return value;
    }),
}));
