// One function per API route. No caching here; `queries.ts` owns that.

import { api } from "../api/client.ts";
import {
  isChannel,
  isServer,
  isServerDetail,
  whole,
  wholeRows,
} from "../api/wire.ts";
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

/** A server row the server answered with, whole, or the request fails. */
export const wholeServer = (value: unknown): Server =>
  whole(value, isServer, "server");
const wholeDetail = (value: unknown): ServerDetail =>
  whole(value, isServerDetail, "server detail");
const wholeChannel = (value: unknown): Channel =>
  whole(value, isChannel, "channel");

export const listServers = (signal?: AbortSignal) =>
  api<unknown>("/servers", { signal }).then((rows) =>
    wholeRows(rows, isServer, "server"),
  );

export const getServer = (id: string, signal?: AbortSignal) =>
  api<unknown>(`/servers/${id}`, { signal }).then(wholeDetail);

export const createServer = (name: string) =>
  api<unknown>("/servers", { method: "POST", body: { name } }).then(
    wholeDetail,
  );

export const updateServer = (id: string, patch: UpdateServerInput) =>
  api<unknown>(`/servers/${id}`, { method: "PATCH", body: patch }).then(
    wholeServer,
  );

/** A square image prepared by `prepareAvatar`; needs `manage_server`. */
export const uploadServerIcon = (id: string, image: Blob) =>
  api<unknown>(`/servers/${id}/icon`, { method: "PUT", body: image }).then(
    wholeServer,
  );

export const removeServerIcon = (id: string) =>
  api<unknown>(`/servers/${id}/icon`, { method: "DELETE" }).then(wholeServer);

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
  api<unknown>(`/servers/${serverId}/channels`, {
    method: "POST",
    body: {
      name: input.name,
      kind: input.kind,
      category_id: input.category_id ?? "",
      ...(input.role_ids !== undefined ? { role_ids: input.role_ids } : {}),
    },
  }).then(wholeChannel);

export const updateChannel = (id: string, patch: UpdateChannelInput) =>
  api<unknown>(`/channels/${id}`, {
    method: "PATCH",
    body: {
      ...(patch.name !== undefined ? { name: patch.name } : {}),
      ...(patch.category_id !== undefined
        ? { category_id: patch.category_id ?? "" }
        : {}),
      ...(patch.role_ids !== undefined ? { role_ids: patch.role_ids } : {}),
    },
  }).then(wholeChannel);

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
  api<unknown>(`/invites/${encodeURIComponent(code)}/join`, {
    method: "POST",
  }).then(wholeServer);

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

/** The owner hands the server to another member (#519); the answer is the
 * server as the caller now sees it, as a member. */
export const transferServer = (serverId: string, userId: string) =>
  api<unknown>(`/servers/${serverId}/transfer`, {
    method: "POST",
    body: { user_id: userId },
  }).then(wholeServer);

/** The link people paste: same origin, web route `/invite/{code}`. */
export function inviteUrl(code: string): string {
  return `${window.location.origin}/invite/${code}`;
}
