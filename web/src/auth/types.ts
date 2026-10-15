export type AvatarDecoration = "ring" | "glow" | "rainbow" | "stars";

/**
 * Optional profile card fields (`ProfileCard` in `api/src/auth/user.rs`).
 * `null` or absent means "not set"; the card falls back to its defaults.
 */
export type ProfileCard = {
  bio?: string | null;
  pronouns?: string | null;
  status_text?: string | null;
  /** `#rrggbb`. */
  accent_color?: string | null;
  /** `#rrggbb`, shown when there is no banner image. */
  banner_color?: string | null;
  /** An uploaded banner (`/api/banners/…`). */
  banner_url?: string | null;
  avatar_decoration?: AvatarDecoration | null;
};

export type User = ProfileCard & {
  id: string;
  email: string;
  name: string;
  avatar_url: string | null;
  created_at: string;
};

/** `GET /api/users/{id}`: another member's card, without the e-mail. */
export type PublicProfile = ProfileCard & {
  id: string;
  name: string;
  avatar_url: string | null;
  created_at: string;
};

export type SessionResponse = {
  user: User | null;
  csrf_token: string;
};

export type LogoutResponse = {
  csrf_token: string;
};

export type ProfilePatch = {
  name?: string;
  /** Empty string clears the avatar. */
  avatar_url?: string;
  /** Empty strings clear the card fields below. */
  bio?: string;
  pronouns?: string;
  status_text?: string;
  accent_color?: string;
  banner_color?: string;
  /** Only `""` (remove) or the current upload. */
  banner_url?: string;
  avatar_decoration?: AvatarDecoration | "";
};
