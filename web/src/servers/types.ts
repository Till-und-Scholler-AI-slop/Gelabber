// Wire shapes of `/api/servers/*` (see `api/src/servers`).

export type Permission =
  | "manage_server"
  | "manage_channels"
  | "manage_messages"
  | "send_messages"
  | "send_files"
  | "join_voice"
  | "go_live";

export type Role = "owner" | "member";

export type ChannelKind = "text" | "voice";

/** A server as the signed-in user sees it: the sidebar list item. */
export type Server = {
  id: string;
  name: string;
  owner_id: string;
  created_at: string;
  role: Role;
  /** The caller's effective flags (everything for the owner). */
  permissions: Permission[];
  /** The mask every non-owner member holds. */
  member_permissions: Permission[];
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
};

export type Member = {
  user_id: string;
  name: string;
  avatar_url: string | null;
  joined_at: string;
  role: Role;
};

export type ServerDetail = Server & {
  categories: Category[];
  channels: Channel[];
  members: Member[];
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
  server: { id: string; name: string; member_count: number };
  expires_at: string | null;
  /** The caller is already in this server. */
  member: boolean;
};

export type CreateChannelInput = {
  name: string;
  kind: ChannelKind;
  category_id: string | null;
};

export type UpdateChannelInput = {
  name?: string;
  /** `null` detaches the channel from its category. */
  category_id?: string | null;
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
