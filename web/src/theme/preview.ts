// A theme tried on from the gallery: shown across the whole app, but never
// written to the account or the first-paint cache until the user keeps it.

import { create } from "zustand";

export const useThemePreview = create<{ id: string | null }>(() => ({
  id: null,
}));

export function previewTheme(id: string): void {
  useThemePreview.setState({ id });
}

export function endThemePreview(): void {
  useThemePreview.setState({ id: null });
}

/** The theme the app shows: a preview, else the account's active theme. */
export function displayedThemeId(active: string, preview: string | null) {
  return preview ?? active;
}

/**
 * The theme the gallery tries on: the picked card, unless it already is the
 * account's theme or a draft is open in the editor. `picked` is null while
 * the gallery follows the account, so a sync never brings back an old one.
 */
export function galleryPreview(
  picked: string | null,
  active: string,
  drafting: boolean,
): string | null {
  return drafting || picked === null || picked === active ? null : picked;
}

/** Whether an Esc press ends the preview: not when a dialog, a menu or a
 * field already handled it. */
export function escapeEndsPreview(event: {
  key: string;
  defaultPrevented: boolean;
  target: EventTarget | null;
}): boolean {
  if (event.key !== "Escape" || event.defaultPrevented) return false;
  const target = event.target as Element | null;
  return !target?.closest?.("dialog, input, textarea, select");
}
