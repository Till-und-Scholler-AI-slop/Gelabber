import type { CSSProperties } from "react";

import { isUploadedBanner, safeColor } from "../auth/rules.ts";
import type {
  AvatarDecoration,
  ProfileCard,
  PublicProfile,
} from "../auth/types.ts";
import type { CardPerson } from "./ProfileCard.tsx";

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
 * color, else the accent color, else the theme default (`null`). Only an
 * upload path the API serves itself becomes an image, like `safeColor` for
 * colors: a second bolt behind the server's check.
 */
export function bannerOf(
  card: ProfileCard,
): { image: string } | { color: string } | null {
  const image = card.banner_url?.trim();
  if (image && isUploadedBanner(image)) return { image };
  const color = safeColor(card.banner_color) ?? safeColor(card.accent_color);
  return color ? { color } : null;
}

/**
 * What a card shows for `userId`: the parts already known (member row, DM
 * peer), overlaid by the fetched card once it is there. A cached card of
 * someone else (the dialog switched people) is ignored.
 */
export function withCard(
  known: CardPerson,
  card: PublicProfile | undefined,
  userId: string,
): CardPerson {
  return card?.id === userId ? { ...known, ...card } : known;
}
