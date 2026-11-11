// One function per API route. No caching here; `queries.ts` owns that.

import { api } from "../api/client.ts";
import type {
  Ban,
  Category,
  Channel,
  CreateChannelInput,
  CreateInviteInput,
  CreateRoleInput,
  Invite,
  InvitePreview,
  MemberRoles,
  Server,
  ServerDetail,
  ServerRole,
  UpdateChannelInput,
  UpdateRoleInput,
  UpdateServerInput,
} from "./types.ts";

/** `roles` came with v0.10; a detail without it has no custom roles. */
const withRoles = (detail: ServerDetail): ServerDetail => ({
  ...detail,
  roles: detail.roles ?? [],
});

export const listServers = (signal?: AbortSignal) =>
  api<Server[]>("/servers", { signal });

export const getServer = (id: string, signal?: AbortSignal) =>
  api<ServerDetail>(`/servers/${id}`, { signal }).then(withRoles);

export const createServer = (name: string) =>
  api<ServerDetail>("/servers", { method: "POST", body: { name } }).then(
    withRoles,
  );

export const updateServer = (id: string, patch: UpdateServerInput) =>
  api<Server>(`/servers/${id}`, { method: "PATCH", body: patch });

/** A square image prepared by `prepareAvatar`; needs `manage_server`. */
export const uploadServerIcon = (id: string, image: Blob) =>
  api<Server>(`/servers/${id}/icon`, { method: "PUT", body: image });

export const removeServerIcon = (id: string) =>
  api<Server>(`/servers/${id}/icon`, { method: "DELETE" });

export const deleteServer = (id: string) =>
  api<null>(`/servers/${id}`, { method: "DELETE" });

export const leaveServer = (id: string) =>
  api<null>(`/servers/${id}/leave`, { method: "POST" });

export const createCategory = (serverId: string, name: string) =>
  api<Category>(`/servers/${serverId}/categories`, {
    method: "POST",
    body: { name },
  });

export const updateCategory = (id: string, name: string) =>
  api<Category>(`/categories/${id}`, { method: "PATCH", body: { name } });

export const deleteCategory = (id: string) =>
  api<null>(`/categories/${id}`, { method: "DELETE" });

export const createChannel = (serverId: string, input: CreateChannelInput) =>
  api<Channel>(`/servers/${serverId}/channels`, {
    method: "POST",
    body: {
      name: input.name,
      kind: input.kind,
      category_id: input.category_id ?? "",
      ...(input.role_ids !== undefined ? { role_ids: input.role_ids } : {}),
    },
  });

export const updateChannel = (id: string, patch: UpdateChannelInput) =>
  api<Channel>(`/channels/${id}`, {
    method: "PATCH",
    body: {
      ...(patch.name !== undefined ? { name: patch.name } : {}),
      ...(patch.category_id !== undefined
        ? { category_id: patch.category_id ?? "" }
        : {}),
      ...(patch.role_ids !== undefined ? { role_ids: patch.role_ids } : {}),
    },
  });

export const deleteChannel = (id: string) =>
  api<null>(`/channels/${id}`, { method: "DELETE" });

export const listInvites = (serverId: string, signal?: AbortSignal) =>
  api<Invite[]>(`/servers/${serverId}/invites`, { signal });

export const createInvite = (serverId: string, input: CreateInviteInput) =>
  api<Invite>(`/servers/${serverId}/invites`, { method: "POST", body: input });

export const revokeInvite = (code: string) =>
  api<null>(`/invites/${code}`, { method: "DELETE" });

export const previewInvite = (code: string, signal?: AbortSignal) =>
  api<InvitePreview>(`/invites/${encodeURIComponent(code)}`, { signal });

export const joinInvite = (code: string) =>
  api<Server>(`/invites/${encodeURIComponent(code)}/join`, { method: "POST" });

export const kickMember = (serverId: string, userId: string) =>
  api<null>(`/servers/${serverId}/kick`, {
    method: "POST",
    body: { user_id: userId },
  });

export const banMember = (serverId: string, userId: string) =>
  api<null>(`/servers/${serverId}/ban`, {
    method: "POST",
    body: { user_id: userId },
  });

export const listBans = (serverId: string, signal?: AbortSignal) =>
  api<Ban[]>(`/servers/${serverId}/bans`, { signal });

export const unbanMember = (serverId: string, userId: string) =>
  api<null>(`/servers/${serverId}/bans/${userId}`, { method: "DELETE" });

/** Needs `manage_server`; the role lands after the others. */
export const createRole = (serverId: string, input: CreateRoleInput) =>
  api<ServerRole>(`/servers/${serverId}/roles`, {
    method: "POST",
    body: input,
  });

export const updateRole = (id: string, patch: UpdateRoleInput) =>
  api<ServerRole>(`/roles/${id}`, { method: "PATCH", body: patch });

export const deleteRole = (id: string) =>
  api<null>(`/roles/${id}`, { method: "DELETE" });

/** Full replacement of a member's custom roles. */
export const setMemberRoles = (
  serverId: string,
  userId: string,
  roleIds: string[],
) =>
  api<MemberRoles>(`/servers/${serverId}/members/${userId}/roles`, {
    method: "PUT",
    body: { role_ids: roleIds },
  });

/** The link people paste: same origin, web route `/invite/{code}`. */
export function inviteUrl(code: string): string {
  return `${window.location.origin}/invite/${code}`;
}
