import { useSyncExternalStore } from "react";

import { readableInk } from "../theme/model.ts";

export type PageTone = "light" | "dark";

/**
 * Whether a page background is light or dark, by its luminance: dark where
 * light ink reads better on it. `fallbackDark` decides for a colour that is
 * not `#rrggbb` (no theme applied yet).
 */
export function toneOf(background: string, fallbackDark: boolean): PageTone {
  const colour = background.trim();
  if (!/^#[\da-f]{6}$/i.test(colour)) return fallbackDark ? "dark" : "light";
  return readableInk(colour) === "#ffffff" ? "dark" : "light";
}

function snapshot(): PageTone {
  const root = document.documentElement;
  return toneOf(
    root.style.getPropertyValue("--lr-bg"),
    root.classList.contains("dark"),
  );
}

function subscribe(onChange: () => void): () => void {
  // Themes are applied as inline variables and the `dark` class on <html>.
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["style", "class"],
  });
  return () => observer.disconnect();
}

/** The current theme's tone, following theme changes. */
export function usePageTone(): PageTone {
  return useSyncExternalStore(subscribe, snapshot, () => "light");
}
