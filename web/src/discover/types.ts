// The server directory "Entdecken" (v0.10): what `/api/directory…` and the
// listing and join-request routes of a server answer.

/** One of the backend's fixed languages (`directory/languages.rs`). The
 * name is its endonym, the same in every interface language. */
export type DirectoryLanguage = {
  code: string;
  name: string;
  /** Listings the caller can see in this language. */
  listings: number;
};

/** `GET /api/directory`. Off, `languages` is empty. */
export type DirectoryStatus = {
  enabled: boolean;
  languages: DirectoryLanguage[];
};

export type JoinMode = "open" | "request";

/** The caller's own request: `declined` only while its cooldown lasts. */
export type RequestState = "pending" | "declined";

/** A listed server as anyone signed in sees it: no owner, no channels. */
export type Listing = {
  server_id: string;
  name: string;
  icon_url: string | null;
  description: string;
  language: string;
  member_count: number;
  join_mode: JoinMode;
  /** The caller is a member already. */
  member: boolean;
  request: RequestState | null;
  /** While `request` is `declined`: when the caller may ask again. */
  retry_at: string | null;
};

/** `GET /api/directory/listings`: 30 at a time, `next_offset` null on the
 * last page. */
export type ListingPage = {
  listings: Listing[];
  next_offset: number | null;
};

export type ListingQuery = {
  q?: string;
  language?: string;
};

/** `GET`/`PUT /api/servers/{id}/listing`: the owner's form. */
export type ListingSettings = {
  listed: boolean;
  description: string;
  /** `null` until the form was saved once. */
  language: string | null;
  join_mode: JoinMode;
  /** The operator took the server out of the directory. */
  delisted: boolean;
  listed_at: string | null;
};

export type ListingSettingsInput = {
  listed: boolean;
  description: string;
  language: string;
  join_mode: JoinMode;
};

/** A pending request in a server's queue. */
export type JoinRequest = {
  user_id: string;
  name: string;
  avatar_url: string | null;
  message: string;
  created_at: string;
};

/** The reasons of operator's `POST /api/servers/{id}/reports`. */
export type ReportReason =
  "spam" | "harassment" | "sexual" | "illegal" | "other";

export type ReportInput = {
  reason: ReportReason;
  /** Required for `other`. */
  details?: string;
};
