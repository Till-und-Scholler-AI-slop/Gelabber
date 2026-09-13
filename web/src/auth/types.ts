export type User = {
  id: string;
  email: string;
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
};
