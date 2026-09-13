// One function per API route. No caching here; `queries.ts` owns that.

import { api } from "../api/client.ts";
import type {
  Category,
  Channel,
  CreateChannelInput,
  CreateInviteInput,
  Invite,
  InvitePreview,
  Server,
  ServerDetail,
  UpdateChannelInput,
  UpdateServerInput,
} from "./types.ts";

export const listServers = (signal?: AbortSignal) =>
  api<Server[]>("/servers", { signal });

export const getServer = (id: string, signal?: AbortSignal) =>
  api<ServerDetail>(`/servers/${id}`, { signal });

export const createServer = (name: string) =>
  api<ServerDetail>("/servers", { method: "POST", body: { name } });

export const updateServer = (id: string, patch: UpdateServerInput) =>
  api<Server>(`/servers/${id}`, { method: "PATCH", body: patch });

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

/** The link people paste: same origin, web route `/invite/{code}`. */
export function inviteUrl(code: string): string {
  return `${window.location.origin}/invite/${code}`;
}
