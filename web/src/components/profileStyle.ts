import type { CSSProperties } from "react";

import { safeColor } from "../auth/rules.ts";
import type { AvatarDecoration, ProfileCard } from "../auth/types.ts";

export const DECORATION_LABEL: Record<AvatarDecoration, string> = {
  ring: "Ring",
  glow: "Leuchten",
  rainbow: "Regenbogen",
  stars: "Sterne",
};

export function isDecoration(value: unknown): value is AvatarDecoration {
  return typeof value === "string" && value in DECORATION_LABEL;
}

/** CSS custom property for someone's accent color, if they set one. */
export function accentStyle(
  accent: string | null | undefined,
): CSSProperties | undefined {
  const color = safeColor(accent);
  return color ? ({ "--profile-accent": color } as CSSProperties) : undefined;
}

/**
 * What the card's banner strip shows: the uploaded image, else the banner
 * color, else the accent color, else the theme default (`null`).
 */
export function bannerOf(
  card: ProfileCard,
): { image: string } | { color: string } | null {
  if (card.banner_url) return { image: card.banner_url };
  const color = safeColor(card.banner_color) ?? safeColor(card.accent_color);
  return color ? { color } : null;
}
