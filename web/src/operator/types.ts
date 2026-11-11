// What `/api/operator/*` serves (backend/api/src/operator), the instance
// operators' area "Betrieb" (v0.10, #517).

export type RegistrationMode = "open" | "invitation";

/** `GET /api/operator`: the area's bootstrap and the reports badge. */
export type OperatorOverview = {
  registration: RegistrationMode;
  /** Account mail is configured, so an invitation can be mailed. */
  mail: boolean;
  open_reports: number;
};

export type Person = { id: string; name: string };

/** A list page: 50 rows from `offset`, and how many match in all. */
export type Page = { total: number };

// ---------------------------------------------------------------------------
// Konten

export type AccountStatus = "active" | "pending" | "disabled";
export type AccountFilter = "all" | "disabled" | "pending";

export type Account = {
  id: string;
  name: string;
  email: string;
  avatar_url: string | null;
  created_at: string;
  /** `pending`: registered, address not confirmed yet. */
  status: AccountStatus;
  disabled_at: string | null;
  disabled_reason: string | null;
  /** Named in `API_OPERATOR_EMAILS`; cannot be disabled here. */
  operator: boolean;
  owned_servers: number;
  /** Every server the account is a member of, its own included. */
  member_servers: number;
  live_sessions: number;
};

export type AccountPage = Page & { accounts: Account[] };

export type AccountQuery = {
  q?: string;
  filter?: AccountFilter;
  offset?: number;
};

// ---------------------------------------------------------------------------
// Server

/** `listed` while the owner shows it in the directory (discovery's
 * `server_listings`), `delisted` while an operator took it out. */
export type DirectoryState = "off" | "listed" | "delisted";
export type ServerFilter = "all" | "listed" | "reported" | "blocked";

export type ServerAdmin = {
  id: string;
  name: string;
  icon_url: string | null;
  created_at: string;
  owner: Person & { email: string };
  members: number;
  /** Attachments of sent messages; icons and previews are not counted. */
  storage_bytes: number;
  directory: DirectoryState;
  open_reports: number;
  blocked_at: string | null;
  blocked_reason: string | null;
};

export type ServerAdminPage = Page & { servers: ServerAdmin[] };

export type ServerQuery = {
  q?: string;
  filter?: ServerFilter;
  offset?: number;
};

// ---------------------------------------------------------------------------
// Meldungen

export type ReportReason =
  "spam" | "harassment" | "sexual" | "illegal" | "other";
export type ReportStatus = "open" | "dismissed" | "actioned";
export type ReportListStatus = "open" | "closed";

export type Report = {
  id: string;
  server: {
    id: string;
    name: string;
    icon_url: string | null;
    blocked: boolean;
    directory: DirectoryState;
  };
  /** `null` once the reporting account is gone. */
  reporter: Person | null;
  reason: ReportReason;
  details: string;
  status: ReportStatus;
  created_at: string;
  resolved_at: string | null;
  resolved_by: Person | null;
  resolution_note: string | null;
  /**
   * The directory listing as the reporter saw it when filing; `null` when
   * the report did not come from a listing.
   */
  snapshot: ReportSnapshot | null;
};

/** What a listing showed when it was reported (`reports.snapshot`). */
export type ReportSnapshot = {
  name: string;
  icon_url: string | null;
  description: string;
  /** An ISO 639-1 code from the directory's list. */
  language: string;
  join_mode: "open" | "request";
};

export type ReportPage = Page & { reports: Report[] };

export type ReportQuery = {
  status?: ReportListStatus;
  offset?: number;
};

export type ResolveReportInput = {
  status: "dismissed" | "actioned";
  note?: string;
};

// ---------------------------------------------------------------------------
// Beta-Einladungen

export type InvitationStatus = "open" | "used" | "expired" | "revoked";
export type InvitationFilter = "open" | "used" | "all";

export type Invitation = {
  id: string;
  code: string;
  /** `<public URL>/register?invitation=<code>`. */
  link: string;
  email: string | null;
  note: string | null;
  created_by: Person | null;
  created_at: string;
  expires_at: string | null;
  status: InvitationStatus;
  used_by: Person | null;
  used_at: string | null;
};

export type InvitationPage = Page & { invitations: Invitation[] };

export type InvitationQuery = {
  status?: InvitationFilter;
  offset?: number;
};

export type CreateInvitationInput = {
  email?: string;
  note?: string;
  /** `null`: never expires. */
  expires_in_days: 1 | 7 | 30 | null;
  send_mail?: boolean;
};

// ---------------------------------------------------------------------------
// Auslastung

export type Usage = {
  generated_at: string;
  registration: RegistrationMode;
  accounts: {
    total: number;
    pending: number;
    disabled: number;
    new_7d: number;
    new_30d: number;
    with_live_session: number;
  };
  servers: { total: number; blocked: number; delisted: number };
  messages: { server_24h: number; server_7d: number; dm_7d: number };
  storage: {
    attachment_bytes: number;
    attachment_count: number;
    uploaded_today_bytes: number;
  };
  now: {
    /** Sockets of this API process only. */
    sockets_this_process: number;
    in_voice: number;
    live: number;
  };
};
