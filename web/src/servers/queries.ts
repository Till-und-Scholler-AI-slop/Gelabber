// TanStack Query layer for servers, channels and invites.
//
// Feel rule: the sidebar never waits on the server. Reads come from the
// cache first (prefetched on hover), and every rename/move/delete is applied
// to the cache *before* the request leaves; the server's answer replaces the
// optimistic row, an error rolls it back. Creates insert a pending row with a
// `tmp:` id that is swapped for the real one on success.

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";

import { notifyError } from "../components/toasts.ts";
import * as remote from "./api.ts";
import { slugifyChannelName } from "./rules.ts";
import type {
  Category,
  Channel,
  CreateChannelInput,
  CreateInviteInput,
  Invite,
  Server,
  ServerDetail,
  UpdateChannelInput,
  UpdateServerInput,
} from "./types.ts";

export const serverKeys = {
  all: ["servers"] as const,
  list: () => ["servers", "list"] as const,
  detail: (id: string) => ["servers", "detail", id] as const,
  invites: (id: string) => ["servers", "invites", id] as const,
};

/** Detail data stays fresh for a minute; switching servers is a cache hit. */
const STALE_MS = 60_000;

const TMP_PREFIX = "tmp:";

/** A row that exists only in this tab until the API confirms it. */
export function isPendingId(id: string): boolean {
  return id.startsWith(TMP_PREFIX);
}

function tmpId(): string {
  return `${TMP_PREFIX}${crypto.randomUUID()}`;
}

function now(): string {
  return new Date().toISOString();
}

export function useServers() {
  return useQuery({
    queryKey: serverKeys.list(),
    queryFn: ({ signal }) => remote.listServers(signal),
    staleTime: STALE_MS,
  });
}

export function useServer(id: string | undefined) {
  return useQuery({
    queryKey: serverKeys.detail(id ?? ""),
    queryFn: ({ signal }) => remote.getServer(id ?? "", signal),
    enabled: id !== undefined,
    staleTime: STALE_MS,
    // A server that vanished (deleted, kicked) should not be retried three
    // times before the page reacts.
    retry: (count, error) =>
      count < 2 &&
      !(
        error instanceof Error &&
        "code" in error &&
        (error.code === "not_found" || error.code === "forbidden")
      ),
  });
}

export function prefetchServer(client: QueryClient, id: string): void {
  void client.prefetchQuery({
    queryKey: serverKeys.detail(id),
    queryFn: ({ signal }) => remote.getServer(id, signal),
    staleTime: STALE_MS,
  });
}

export function useInvites(serverId: string, enabled: boolean) {
  return useQuery({
    queryKey: serverKeys.invites(serverId),
    queryFn: ({ signal }) => remote.listInvites(serverId, signal),
    enabled,
    staleTime: STALE_MS,
  });
}

// ---------------------------------------------------------------------------
// Cache helpers

function toListItem(detail: ServerDetail): Server {
  const {
    id,
    name,
    owner_id,
    created_at,
    role,
    permissions,
    member_permissions,
  } = detail;
  return {
    id,
    name,
    owner_id,
    created_at,
    role,
    permissions,
    member_permissions,
  };
}

function patchList(
  client: QueryClient,
  update: (servers: Server[]) => Server[],
): void {
  client.setQueryData<Server[]>(serverKeys.list(), (current) =>
    current ? update(current) : current,
  );
}

function patchDetail(
  client: QueryClient,
  serverId: string,
  update: (detail: ServerDetail) => ServerDetail,
): void {
  client.setQueryData<ServerDetail>(serverKeys.detail(serverId), (current) =>
    current ? update(current) : current,
  );
}

/** Applies a `Server` (list shape) onto both the list row and the detail. */
function mergeServer(client: QueryClient, server: Server): void {
  patchList(client, (servers) =>
    servers.map((s) => (s.id === server.id ? { ...s, ...server } : s)),
  );
  patchDetail(client, server.id, (detail) => ({ ...detail, ...server }));
}

function removeServer(
  client: QueryClient,
  serverId: string,
  options: { keepDetail?: boolean } = {},
): void {
  patchList(client, (servers) => servers.filter((s) => s.id !== serverId));
  if (!options.keepDetail)
    client.removeQueries({ queryKey: serverKeys.detail(serverId) });
  client.removeQueries({ queryKey: serverKeys.invites(serverId) });
}

type Snapshot = {
  list: Server[] | undefined;
  detail: ServerDetail | undefined;
};

function snapshot(client: QueryClient, serverId: string): Snapshot {
  return {
    list: client.getQueryData<Server[]>(serverKeys.list()),
    detail: client.getQueryData<ServerDetail>(serverKeys.detail(serverId)),
  };
}

function restore(
  client: QueryClient,
  serverId: string,
  snap: Snapshot | undefined,
): void {
  if (!snap) return;
  if (snap.list) client.setQueryData(serverKeys.list(), snap.list);
  if (snap.detail)
    client.setQueryData(serverKeys.detail(serverId), snap.detail);
}

/** Swaps a `tmp:` row for the real one (or drops it on failure). */
function replaceRow<T extends { id: string }>(
  rows: T[],
  tmp: string,
  real: T | null,
): T[] {
  return real
    ? rows.map((row) => (row.id === tmp ? real : row))
    : rows.filter((row) => row.id !== tmp);
}

// ---------------------------------------------------------------------------
// Servers

export function useCreateServer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => remote.createServer(name.trim()),
    onSuccess: (detail) => {
      client.setQueryData(serverKeys.detail(detail.id), detail);
      patchList(client, (servers) => [...servers, toListItem(detail)]);
      // A first server created while the list was still empty/unfetched.
      if (!client.getQueryData(serverKeys.list())) {
        client.setQueryData(serverKeys.list(), [toListItem(detail)]);
      }
    },
  });
}

export function useUpdateServer(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (patch: UpdateServerInput) =>
      remote.updateServer(serverId, patch),
    onMutate: (patch) => {
      const snap = snapshot(client, serverId);
      const apply = <T extends Server>(server: T): T => {
        const next = { ...server };
        if (patch.name !== undefined) next.name = patch.name.trim();
        if (patch.member_permissions !== undefined) {
          next.member_permissions = patch.member_permissions;
          if (server.role === "member")
            next.permissions = patch.member_permissions;
        }
        return next;
      };
      patchList(client, (servers) =>
        servers.map((s) => (s.id === serverId ? apply(s) : s)),
      );
      patchDetail(client, serverId, apply);
      return snap;
    },
    onError: (error, _patch, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
    onSuccess: (server) => mergeServer(client, server),
  });
}

export function useDeleteServer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (serverId: string) => remote.deleteServer(serverId),
    onMutate: (serverId) => {
      const snap = snapshot(client, serverId);
      removeServer(client, serverId);
      return snap;
    },
    onError: (error, serverId, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
  });
}

export function useLeaveServer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (serverId: string) => remote.leaveServer(serverId),
    onMutate: (serverId) => {
      const snap = snapshot(client, serverId);
      removeServer(client, serverId);
      return snap;
    },
    onError: (error, serverId, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
  });
}

/**
 * Called when a read comes back 404: the server is gone for this user.
 * `keepDetail` leaves the (failed) detail query alone while a component
 * still observes it; removing it would trigger an immediate refetch.
 */
export function forgetServer(
  client: QueryClient,
  serverId: string,
  options: { keepDetail?: boolean } = {},
): void {
  removeServer(client, serverId, options);
}

// ---------------------------------------------------------------------------
// Categories

export function useCreateCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => remote.createCategory(serverId, name.trim()),
    onMutate: (name) => {
      const tmp = tmpId();
      const pending: Category = {
        id: tmp,
        server_id: serverId,
        name: name.trim(),
        created_at: now(),
      };
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        categories: [...detail.categories, pending],
      }));
      return tmp;
    },
    onSuccess: (category, _name, tmp) =>
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        categories: replaceRow(detail.categories, tmp, category),
      })),
    onError: (error, _name, tmp) => {
      if (tmp)
        patchDetail(client, serverId, (detail) => ({
          ...detail,
          categories: replaceRow(detail.categories, tmp, null),
        }));
      notifyError(error);
    },
  });
}

export function useUpdateCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      remote.updateCategory(id, name.trim()),
    onMutate: ({ id, name }) => {
      const snap = snapshot(client, serverId);
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        categories: detail.categories.map((c) =>
          c.id === id ? { ...c, name: name.trim() } : c,
        ),
      }));
      return snap;
    },
    onError: (error, _vars, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
    onSuccess: (category) =>
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        categories: detail.categories.map((c) =>
          c.id === category.id ? category : c,
        ),
      })),
  });
}

export function useDeleteCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => remote.deleteCategory(id),
    onMutate: (id) => {
      const snap = snapshot(client, serverId);
      // Channels stay, uncategorised — exactly what the foreign key does.
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        categories: detail.categories.filter((c) => c.id !== id),
        channels: detail.channels.map((ch) =>
          ch.category_id === id ? { ...ch, category_id: null } : ch,
        ),
      }));
      return snap;
    },
    onError: (error, _id, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
  });
}

// ---------------------------------------------------------------------------
// Channels

export function useCreateChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateChannelInput) =>
      remote.createChannel(serverId, input),
    onMutate: (input) => {
      const tmp = tmpId();
      const pending: Channel = {
        id: tmp,
        server_id: serverId,
        category_id: input.category_id,
        name:
          input.kind === "text"
            ? slugifyChannelName(input.name)
            : input.name.trim(),
        kind: input.kind,
        created_at: now(),
      };
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        channels: [...detail.channels, pending],
      }));
      return tmp;
    },
    onSuccess: (channel, _input, tmp) =>
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        channels: replaceRow(detail.channels, tmp, channel),
      })),
    onError: (error, _input, tmp) => {
      if (tmp)
        patchDetail(client, serverId, (detail) => ({
          ...detail,
          channels: replaceRow(detail.channels, tmp, null),
        }));
      notifyError(error);
    },
  });
}

export function useUpdateChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({ id, patch }: { id: string; patch: UpdateChannelInput }) =>
      remote.updateChannel(id, patch),
    onMutate: ({ id, patch }) => {
      const snap = snapshot(client, serverId);
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        channels: detail.channels.map((ch) => {
          if (ch.id !== id) return ch;
          const next = { ...ch };
          if (patch.name !== undefined)
            next.name =
              ch.kind === "text"
                ? slugifyChannelName(patch.name)
                : patch.name.trim();
          if (patch.category_id !== undefined)
            next.category_id = patch.category_id;
          return next;
        }),
      }));
      return snap;
    },
    onError: (error, _vars, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
    onSuccess: (channel) =>
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        channels: detail.channels.map((ch) =>
          ch.id === channel.id ? channel : ch,
        ),
      })),
  });
}

export function useDeleteChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (id: string) => remote.deleteChannel(id),
    onMutate: (id) => {
      const snap = snapshot(client, serverId);
      patchDetail(client, serverId, (detail) => ({
        ...detail,
        channels: detail.channels.filter((ch) => ch.id !== id),
      }));
      return snap;
    },
    onError: (error, _id, snap) => {
      restore(client, serverId, snap);
      notifyError(error);
    },
  });
}

// ---------------------------------------------------------------------------
// Invites

export function useCreateInvite(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateInviteInput) =>
      remote.createInvite(serverId, input),
    onSuccess: (invite) =>
      client.setQueryData<Invite[]>(serverKeys.invites(serverId), (current) =>
        current ? [invite, ...current] : current,
      ),
  });
}

export function useRevokeInvite(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (code: string) => remote.revokeInvite(code),
    onMutate: (code) => {
      const previous = client.getQueryData<Invite[]>(
        serverKeys.invites(serverId),
      );
      client.setQueryData<Invite[]>(serverKeys.invites(serverId), (current) =>
        current?.filter((invite) => invite.code !== code),
      );
      return previous;
    },
    onError: (error, _code, previous) => {
      if (previous) client.setQueryData(serverKeys.invites(serverId), previous);
      notifyError(error);
    },
  });
}

export function useJoinInvite() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (code: string) => remote.joinInvite(code),
    onSuccess: (server) => {
      patchList(client, (servers) =>
        servers.some((s) => s.id === server.id)
          ? servers
          : [...servers, server],
      );
      // Members and channels are not part of the join answer.
      void client.invalidateQueries({ queryKey: serverKeys.detail(server.id) });
    },
  });
}
