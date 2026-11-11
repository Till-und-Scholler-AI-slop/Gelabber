import type { Locale } from "../i18n/locale.ts";

export type AvatarDecoration = "ring" | "glow" | "rainbow" | "stars";

/**
 * Optional profile card fields (`ProfileCard` in `backend/api/src/auth/user.rs`).
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
  /** The account's language: the interface, push texts and mail. */
  locale: Locale;
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
  /** An instance operator, who may open "Betrieb" (v0.10). */
  operator?: boolean;
};

/**
 * `POST /api/auth/register` while the server sends account mail: nobody is
 * signed in until the mailed link is opened.
 */
export type VerificationSentResponse = {
  user: null;
  csrf_token: string;
  verification_sent: true;
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

/** A sign-in provider the instance offers (`GET /api/auth/providers`). */
export type AuthProvider = {
  /** Its route segment, e.g. `"google"`. */
  id: string;
  /** Shown on buttons and in "Sicherheit", e.g. `"Google"`. */
  name: string;
};

/** `GET /api/auth/methods`: how the signed-in account can sign in. */
export type SignInMethods = {
  /** Whether the account has a password. */
  password: boolean;
  providers: (AuthProvider & {
    /** `false`: the operator turned it off; it can still be disconnected. */
    configured: boolean;
    connected: { email: string; since: string } | null;
  })[];
};

/** `POST /api/auth/oidc/{provider}/start`, web variant. */
export type OidcStartBody = {
  purpose: "sign_in" | "link";
  /** Where to continue after signing in; the API drops unsafe targets. */
  redirect?: string;
  /** `link` only, required when the account has a password. */
  current_password?: string;
  /** A beta invitation's code, which admits a new account. */
  invitation?: string;
};

/** `POST /api/auth/oidc/pending`: a first sign-in waiting for its name. */
export type OidcPending = {
  provider: AuthProvider;
  email: string;
  /** The provider's name for the person, a suggestion. */
  name: string | null;
};

/** `POST /api/auth/oidc/complete`: the new account, signed in. */
export type OidcCompleteResponse = SessionResponse & {
  user: User;
  /** Where the sign-in was headed, e.g. an invite. */
  redirect: string | null;
};
