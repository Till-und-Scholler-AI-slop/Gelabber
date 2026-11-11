// Wire shapes of `/api/servers/*` (see `backend/api/src/servers`).

import type { ProfileCard } from "../auth/types.ts";

export type Permission =
  | "manage_server"
  | "manage_channels"
  | "manage_messages"
  | "send_messages"
  | "send_files"
  | "join_voice"
  | "go_live"
  | "mention_everyone"
  | "moderate_members";

export type Role = "owner" | "member";

export type ChannelKind = "text" | "voice";

/** A server as the signed-in user sees it: the sidebar list item. */
export type Server = {
  id: string;
  name: string;
  owner_id: string;
  /** Uploaded icon (`/api/server-icons/…`); without one the initials show.
   * Optional: lists cached before the field existed lack it. */
  icon_url?: string | null;
  created_at: string;
  role: Role;
  /** The caller's effective flags (everything for the owner). */
  permissions: Permission[];
  /** The mask every non-owner member holds. */
  member_permissions: Permission[];
  /** An operator blocked the server: it stays listed, but everything else
   * answers `403 server_blocked`. Optional: older APIs lack it. */
  blocked?: boolean;
};

export type Category = {
  id: string;
  server_id: string;
  name: string;
  created_at: string;
};

export type Channel = {
  id: string;
  server_id: string;
  category_id: string | null;
  name: string;
  kind: ChannelKind;
  created_at: string;
  /** Seen only by the owner and holders of one of `role_ids`. A limited
   * channel whose last role was deleted is the owner's alone. */
  limited: boolean;
  /** The roles a limited channel is for; empty when it is not limited. */
  role_ids: string[];
};

export type Member = Pick<
  ProfileCard,
  "status_text" | "accent_color" | "avatar_decoration"
> & {
  user_id: string;
  name: string;
  avatar_url: string | null;
  joined_at: string;
  role: Role;
  /** The member's custom roles, by position. */
  role_ids: string[];
};

/** The six role colours, as the API stores them. */
export type RoleColor =
  "amber" | "blue" | "green" | "pink" | "violet" | "coral";

/** A custom role. "Alle Mitglieder" (`member_permissions`) and the owner
 * are fixed and never one of these. */
export type ServerRole = {
  id: string;
  server_id: string;
  name: string;
  color: RoleColor;
  permissions: Permission[];
  position: number;
  created_at: string;
};

export type ServerDetail = Server & {
  categories: Category[];
  channels: Channel[];
  members: Member[];
  /** Custom roles, by position. */
  roles: ServerRole[];
};

export type Invite = {
  code: string;
  server_id: string;
  created_by: string;
  created_at: string;
  expires_at: string | null;
  max_uses: number | null;
  uses: number;
};

export type InvitePreview = {
  code: string;
  server: {
    id: string;
    name: string;
    icon_url?: string | null;
    member_count: number;
  };
  expires_at: string | null;
  /** The caller is already in this server. */
  member: boolean;
};

export type CreateChannelInput = {
  name: string;
  kind: ChannelKind;
  category_id: string | null;
  /** Limits the new channel to these roles; absent or `[]`: everyone. */
  role_ids?: string[];
};

export type UpdateChannelInput = {
  name?: string;
  /** `null` detaches the channel from its category. */
  category_id?: string | null;
  /** `[]` opens the channel to everyone; a list limits it to those roles. */
  role_ids?: string[];
};

export type UpdateServerInput = {
  name?: string;
  member_permissions?: Permission[];
};

export type CreateInviteInput = {
  max_uses?: number;
  expires_in_hours?: number;
};

export type Ban = {
  user_id: string;
  name: string;
  avatar_url: string | null;
  banned_by: string;
  banned_at: string;
};

export type CreateRoleInput = {
  name: string;
  color: RoleColor;
  permissions?: Permission[];
};

/** Full replacement of the fields given. */
export type UpdateRoleInput = {
  name?: string;
  color?: RoleColor;
  permissions?: Permission[];
};

export type MemberRoles = {
  user_id: string;
  role_ids: string[];
};
