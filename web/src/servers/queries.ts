// TanStack Query layer for servers, channels and invites.
//
// Feel rule: the sidebar never waits on the server. Reads come from the
// cache first (prefetched on hover), and every rename/move/delete is applied
// to the cache *before* the request leaves; the server's answer replaces the
// optimistic row, an error rolls it back. Creates insert a pending row with a
// `tmp:` id that is swapped for the real one on success.

import {
  mutationOptions,
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useRef } from "react";

import { ApiError } from "../api/client.ts";
import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { servers } from "../i18n/catalogs/servers.ts";
import { randomUuid } from "../lib/uuid.ts";
import * as remote from "./api.ts";
import { effectivePermissions } from "./roles.ts";
import { slugifyChannelName } from "./rules.ts";
import type {
  Ban,
  Category,
  Channel,
  CreateChannelInput,
  CreateInviteInput,
  CreateRoleInput,
  Invite,
  Member,
  Permission,
  Server,
  ServerDetail,
  ServerRole,
  UpdateChannelInput,
  UpdateRoleInput,
  UpdateServerInput,
} from "./types.ts";

export const serverKeys = {
  all: (userId: string, generation: number) =>
    ["user", userId, generation, "servers"] as const,
  list: (userId: string, generation: number) =>
    ["user", userId, generation, "servers", "list"] as const,
  detail: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "servers", "detail", id] as const,
  invites: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "servers", "invites", id] as const,
  bans: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "servers", "bans", id] as const,
  invite: (userId: string, generation: number, code: string) =>
    ["user", userId, generation, "invite", code] as const,
};

/** Detail data stays fresh for a minute; switching servers is a cache hit. */
const STALE_MS = 60_000;

const TMP_PREFIX = "tmp:";

/** A row that exists only in this tab until the API confirms it. */
export function isPendingId(id: string): boolean {
  return id.startsWith(TMP_PREFIX);
}

function tmpId(): string {
  return `${TMP_PREFIX}${randomUuid()}`;
}

function now(): string {
  return new Date().toISOString();
}

export function useServers() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: serverKeys.list(userId, generation),
    queryFn: ({ signal }) => remote.listServers(signal),
    enabled: userId.length > 0,
    staleTime: STALE_MS,
  });
}

export function useServer(id: string | undefined) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: serverKeys.detail(userId, generation, id ?? ""),
    queryFn: ({ signal }) => remote.getServer(id ?? "", signal),
    enabled: userId.length > 0 && id !== undefined,
    staleTime: STALE_MS,
    // A server that vanished (deleted, kicked) should not be retried three
    // times before the page reacts.
    retry: (count, error) =>
      count < 2 &&
      !(
        error instanceof Error &&
        "code" in error &&
        (error.code === "not_found" ||
          error.code === "forbidden" ||
          error.code === "server_blocked")
      ),
  });
}

/** The loaded details of several servers (the quick switcher, the people
 * to write to); the ones still loading are left out. */
export function useServerDetails(ids: readonly string[]): ServerDetail[] {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQueries({
    queries: ids.map((id) => ({
      queryKey: serverKeys.detail(userId, generation, id),
      queryFn: ({ signal }: { signal: AbortSignal }) =>
        remote.getServer(id, signal),
      enabled: userId.length > 0,
      staleTime: STALE_MS,
    })),
    combine: (results) => results.flatMap((result) => result.data ?? []),
  });
}

export function prefetchServer(
  client: QueryClient,
  userId: string,
  id: string,
): void {
  const generation = scopeGeneration();
  void client.prefetchQuery({
    queryKey: serverKeys.detail(userId, generation, id),
    queryFn: ({ signal }) => remote.getServer(id, signal),
    staleTime: STALE_MS,
  });
}

/** A server's detail from the cache, else fetched: a Live notice can be
 * about a server whose page was not open yet (voice/liveNotices.ts). */
export function ensureServer(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  id: string,
): Promise<ServerDetail> {
  return client.ensureQueryData({
    queryKey: serverKeys.detail(stamp.userId, stamp.generation, id),
    queryFn: ({ signal }) => remote.getServer(id, signal),
    staleTime: STALE_MS,
  });
}

export function useInvites(serverId: string, enabled: boolean) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: serverKeys.invites(userId, generation, serverId),
    queryFn: ({ signal }) => remote.listInvites(serverId, signal),
    enabled: userId.length > 0 && enabled,
    staleTime: STALE_MS,
  });
}

export function useBans(serverId: string, enabled: boolean) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: serverKeys.bans(userId, generation, serverId),
    queryFn: ({ signal }) => remote.listBans(serverId, signal),
    enabled: userId.length > 0 && enabled,
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
    icon_url,
    created_at,
    role,
    permissions,
    member_permissions,
  } = detail;
  return {
    id,
    name,
    owner_id,
    icon_url,
    created_at,
    role,
    permissions,
    member_permissions,
  };
}

function patchList(
  client: QueryClient,
  userId: string,
  generation: number,
  update: (servers: Server[]) => Server[],
): void {
  client.setQueryData<Server[]>(
    serverKeys.list(userId, generation),
    (current) => (current ? update(current) : current),
  );
}

function patchDetail(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
  update: (detail: ServerDetail) => ServerDetail,
): void {
  client.setQueryData<ServerDetail>(
    serverKeys.detail(userId, generation, serverId),
    (current) => (current ? update(current) : current),
  );
}

/** Applies a `Server` (list shape) onto both the list row and the detail. */
function mergeServer(
  client: QueryClient,
  userId: string,
  generation: number,
  server: Server,
): void {
  patchList(client, userId, generation, (servers) =>
    servers.map((s) => (s.id === server.id ? { ...s, ...server } : s)),
  );
  patchDetail(client, userId, generation, server.id, (detail) => ({
    ...detail,
    ...server,
  }));
}

function removeServer(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
  options: { keepDetail?: boolean; keepList?: boolean } = {},
): void {
  if (!options.keepList) {
    patchList(client, userId, generation, (servers) =>
      servers.filter((s) => s.id !== serverId),
    );
  }
  if (!options.keepDetail) {
    client.removeQueries({
      queryKey: serverKeys.detail(userId, generation, serverId),
    });
  }
  client.removeQueries({
    queryKey: serverKeys.invites(userId, generation, serverId),
  });
  client.removeQueries({
    queryKey: serverKeys.bans(userId, generation, serverId),
  });
}

type Snapshot = {
  list: Server[] | undefined;
  detail: ServerDetail | undefined;
};

function snapshot(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): Snapshot {
  return {
    list: client.getQueryData<Server[]>(serverKeys.list(userId, generation)),
    detail: client.getQueryData<ServerDetail>(
      serverKeys.detail(userId, generation, serverId),
    ),
  };
}

/** Writes the whole snapshot back, wiping anything newer: only for the burst
 * writes that check no later write of theirs is in flight. Other writes undo
 * only their own change (`putBack`, `unpatchChannel`). */
function restore(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
  snap: Snapshot | undefined,
): void {
  if (!snap) return;
  if (snap.list)
    client.setQueryData(serverKeys.list(userId, generation), snap.list);
  if (snap.detail) {
    client.setQueryData(
      serverKeys.detail(userId, generation, serverId),
      snap.detail,
    );
  }
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

const byId = (row: { id: string }) => row.id;

/**
 * Undoes an optimistic removal that failed: the row goes back after the
 * nearest row that stood before it and is still there. Rows that came or
 * went while the request was in flight stay as they are, and a row that is
 * back already is left alone.
 */
function putBack<T>(
  rows: T[],
  before: readonly T[],
  row: T,
  key: (row: T) => string,
): T[] {
  const id = key(row);
  if (rows.some((r) => key(r) === id)) return rows;
  for (let i = before.findIndex((r) => key(r) === id) - 1; i >= 0; i--) {
    const anchor = key(before[i]!);
    const at = rows.findIndex((r) => key(r) === anchor);
    if (at >= 0) return [...rows.slice(0, at + 1), row, ...rows.slice(at + 1)];
  }
  return [row, ...rows];
}

/** A removal that failed because its target is already gone (a member who
 * left, a channel deleted elsewhere): there is nothing to put back. */
const targetGone = (error: unknown) =>
  error instanceof ApiError && error.code === "not_found";

// ---------------------------------------------------------------------------
// Servers

export function useCreateServer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => remote.createServer(name.trim()),
    onMutate: () => takeStamp(),
    onSuccess: (detail, _name, stamp) => {
      if (!stampHolds(stamp)) return;
      const userId = stamp.userId;
      const generation = stamp.generation;
      client.setQueryData(
        serverKeys.detail(userId, generation, detail.id),
        detail,
      );
      patchList(client, userId, generation, (servers) => [
        ...servers,
        toListItem(detail),
      ]);
      // A first server created while the list was still empty/unfetched.
      if (!client.getQueryData(serverKeys.list(userId, generation))) {
        client.setQueryData(serverKeys.list(userId, generation), [
          toListItem(detail),
        ]);
      }
    },
  });
}

/** The cached detail, if any — the freshest state including optimistic writes. */
export function readServer(
  client: QueryClient,
  userId: string,
  serverId: string,
): ServerDetail | undefined {
  return client.getQueryData<ServerDetail>(
    serverKeys.detail(userId, scopeGeneration(), serverId),
  );
}

/**
 * Consecutive PATCHes on one server (six permission checkboxes clicked in a
 * row) overlap. Each is applied optimistically at once; only the *last* one
 * in flight may write the server's answer back, otherwise an earlier reply
 * would undo a later click. If any PATCH in such a burst fails, the burst
 * ends with a refetch instead of a rollback, because the snapshot of the
 * failed one would also wipe the clicks that came after it.
 */
export function useUpdateServer(serverId: string) {
  const client = useQueryClient();
  const userId = useUserId();
  const mutationKey = ["servers", "update", userId, serverId];
  const resync = useRef(false);
  const lastInFlight = () => client.isMutating({ mutationKey }) <= 1;
  return useMutation({
    mutationKey,
    mutationFn: (patch: UpdateServerInput) =>
      remote.updateServer(serverId, patch),
    onMutate: (patch) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      // A member's flags are the new base role plus their own roles.
      const mine =
        patch.member_permissions !== undefined && snap.detail
          ? ownPermissions(
              { ...snap.detail, member_permissions: patch.member_permissions },
              stamp.userId,
            )
          : undefined;
      const apply = <T extends Server>(server: T): T => {
        const next = { ...server };
        if (patch.name !== undefined) next.name = patch.name.trim();
        if (patch.member_permissions !== undefined) {
          next.member_permissions = patch.member_permissions;
          if (server.role === "member")
            next.permissions = mine ?? patch.member_permissions;
        }
        return next;
      };
      patchList(client, stamp.userId, stamp.generation, (servers) =>
        servers.map((s) => (s.id === serverId ? apply(s) : s)),
      );
      patchDetail(client, stamp.userId, stamp.generation, serverId, apply);
      return { ...stamp, snap };
    },
    onError: (error, _patch, ctx) => {
      if (!stampHolds(ctx)) return;
      if (lastInFlight() && !resync.current)
        restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      else resync.current = true;
      notifyError(error, translate(servers, "error.serverNotSaved"));
    },
    onSuccess: (server, _patch, ctx) => {
      if (!stampHolds(ctx)) return;
      if (lastInFlight())
        mergeServer(client, ctx.userId, ctx.generation, server);
    },
    onSettled: (_data, _error, _patch, ctx) => {
      if (!stampHolds(ctx)) return;
      const failed = lastInFlight() && resync.current;
      if (failed) resync.current = false;
      settleBurst(client, ctx, serverId, failed);
    },
  });
}

/**
 * Sets (`Blob`, prepared by `prepareAvatar`) or removes (`null`) the
 * server icon. Not optimistic: the new URL only exists once the server
 * stored the file.
 */
export function useServerIcon(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (image: Blob | null) => {
      const stamp = takeStamp();
      const server = image
        ? await remote.uploadServerIcon(serverId, image)
        : await remote.removeServerIcon(serverId);
      return { server, stamp };
    },
    onSuccess: ({ server, stamp }) => {
      if (stamp && stampHolds(stamp))
        mergeServer(client, stamp.userId, stamp.generation, server);
    },
  });
}

/** Deletes and leaves of servers share this key, so the list is read again
 * only once none of them is in flight. */
const removeServerKey = ["servers", "remove"];
const resyncServerList = new Set<string>();

/**
 * A delete or leave that failed: the server's row goes back into the list.
 * While it was out, events had nothing to patch: its detail shows the
 * snapshot again at once and is read anew, and the list is read again once
 * no other delete or leave is in flight, which an earlier read could undo.
 */
function undoRemoveServer(
  client: QueryClient,
  ctx: { userId: string; generation: number; snap: Snapshot },
  serverId: string,
  error: unknown,
): void {
  if (targetGone(error)) return;
  const { userId, generation, snap } = ctx;
  const before = snap.list;
  const row = before?.find((s) => s.id === serverId);
  if (before && row)
    patchList(client, userId, generation, (list) =>
      putBack(list, before, row, byId),
    );
  const key = serverKeys.detail(userId, generation, serverId);
  if (snap.detail && client.getQueryData(key) === undefined) {
    client.setQueryData(key, snap.detail);
    void client.invalidateQueries({ queryKey: key });
  }
  resyncServerList.add(`${userId}/${generation}`);
}

function settleRemoveServer(
  client: QueryClient,
  ctx: { userId: string; generation: number },
): void {
  const id = `${ctx.userId}/${ctx.generation}`;
  if (
    client.isMutating({ mutationKey: removeServerKey }) > 1 ||
    !resyncServerList.delete(id)
  )
    return;
  void client.invalidateQueries({
    queryKey: serverKeys.list(ctx.userId, ctx.generation),
  });
}

export function deleteServerOptions(client: QueryClient) {
  return mutationOptions({
    mutationKey: removeServerKey,
    mutationFn: (serverId: string) => remote.deleteServer(serverId),
    onMutate: (serverId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      removeServer(client, stamp.userId, stamp.generation, serverId);
      return { ...stamp, snap };
    },
    onError: (error, serverId, ctx) => {
      if (!stampHolds(ctx)) return;
      undoRemoveServer(client, ctx, serverId, error);
      notifyError(error, translate(servers, "error.serverNotDeleted"));
    },
    onSettled: (_data, _error, _serverId, ctx) => {
      if (stampHolds(ctx)) settleRemoveServer(client, ctx);
    },
  });
}

export function useDeleteServer() {
  const client = useQueryClient();
  return useMutation(deleteServerOptions(client));
}

export function leaveServerOptions(client: QueryClient) {
  return mutationOptions({
    mutationKey: removeServerKey,
    mutationFn: (serverId: string) => remote.leaveServer(serverId),
    onMutate: (serverId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      removeServer(client, stamp.userId, stamp.generation, serverId);
      return { ...stamp, snap };
    },
    onError: (error, serverId, ctx) => {
      if (!stampHolds(ctx)) return;
      undoRemoveServer(client, ctx, serverId, error);
      notifyError(error, translate(servers, "error.serverNotLeft"));
    },
    onSettled: (_data, _error, _serverId, ctx) => {
      if (stampHolds(ctx)) settleRemoveServer(client, ctx);
    },
  });
}

export function useLeaveServer() {
  const client = useQueryClient();
  return useMutation(leaveServerOptions(client));
}

/**
 * An operator blocked the server: it stays in the list, marked, and its
 * cached details go, so the next read meets `server_blocked`.
 */
export function markServerBlocked(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): void {
  patchList(client, userId, generation, (servers) =>
    servers.map((s) => (s.id === serverId ? { ...s, blocked: true } : s)),
  );
  removeServer(client, userId, generation, serverId, { keepList: true });
}

/**
 * Called when a read comes back 404: the server is gone for this user.
 * `keepDetail` leaves the (failed) detail query alone while a component
 * still observes it; removing it would trigger an immediate refetch.
 */
export function forgetServer(
  client: QueryClient,
  userId: string,
  serverId: string,
  options: { keepDetail?: boolean } = {},
): void {
  removeServer(client, userId, scopeGeneration(), serverId, options);
}

// ---------------------------------------------------------------------------
// Categories

export function useCreateCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (name: string) => remote.createCategory(serverId, name.trim()),
    onMutate: (name) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const tmp = tmpId();
      const pending: Category = {
        id: tmp,
        server_id: serverId,
        name: name.trim(),
        created_at: now(),
      };
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          categories: [...detail.categories, pending],
        }),
      );
      return { ...stamp, tmp };
    },
    onSuccess: (category, _name, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        categories: replaceRow(detail.categories, ctx.tmp, category),
      }));
    },
    onError: (error, _name, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        categories: replaceRow(detail.categories, ctx.tmp, null),
      }));
      notifyError(error, translate(servers, "error.categoryNotCreated"));
    },
  });
}

export function updateCategoryOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: ({ id, name }: { id: string; name: string }) =>
      remote.updateCategory(id, name.trim()),
    onMutate: ({ id, name }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          categories: detail.categories.map((c) =>
            c.id === id ? { ...c, name: name.trim() } : c,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, { id, name }, ctx) => {
      if (!stampHolds(ctx)) return;
      // The old name comes back unless a later rename replaced ours.
      const was = ctx.snap.detail?.categories.find((c) => c.id === id);
      if (was)
        patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          categories: detail.categories.map((c) =>
            c.id === id && c.name === name.trim()
              ? { ...c, name: was.name }
              : c,
          ),
        }));
      notifyError(error, translate(servers, "error.categoryNotSaved"));
    },
    onSuccess: (category, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        categories: detail.categories.map((c) =>
          c.id === category.id ? category : c,
        ),
      }));
    },
  });
}

export function useUpdateCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation(updateCategoryOptions(client, serverId));
}

export function deleteCategoryOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: (id: string) => remote.deleteCategory(id),
    onMutate: (id) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      // Channels stay, uncategorised — exactly what the foreign key does.
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          categories: detail.categories.filter((c) => c.id !== id),
          channels: detail.channels.map((ch) =>
            ch.category_id === id ? { ...ch, category_id: null } : ch,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, id, ctx) => {
      if (!stampHolds(ctx)) return;
      const before = ctx.snap.detail;
      const was = before?.categories.find((c) => c.id === id);
      if (before && was && !targetGone(error)) {
        // Only the channels we uncategorised and nobody moved since go back.
        const ours = new Set(
          before.channels.filter((ch) => ch.category_id === id).map(byId),
        );
        patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          categories: putBack(detail.categories, before.categories, was, byId),
          channels: detail.channels.map((ch) =>
            ours.has(ch.id) && ch.category_id === null
              ? { ...ch, category_id: id }
              : ch,
          ),
        }));
      }
      notifyError(error, translate(servers, "error.categoryNotDeleted"));
    },
  });
}

export function useDeleteCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation(deleteCategoryOptions(client, serverId));
}

// ---------------------------------------------------------------------------
// Channels

export function useCreateChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateChannelInput) =>
      remote.createChannel(serverId, input),
    onMutate: (input) => {
      const stamp = takeStamp();
      if (!stamp) return;
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
        limited: (input.role_ids?.length ?? 0) > 0,
        role_ids: input.role_ids ?? [],
      };
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          channels: [...detail.channels, pending],
        }),
      );
      return { ...stamp, tmp };
    },
    onSuccess: (channel, _input, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        channels: replaceRow(detail.channels, ctx.tmp, channel),
      }));
    },
    onError: (error, _input, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        channels: replaceRow(detail.channels, ctx.tmp, null),
      }));
      notifyError(error, translate(servers, "error.channelNotCreated"));
    },
  });
}

/** A channel as the PATCH will leave it. */
function patchChannel(ch: Channel, patch: UpdateChannelInput): Channel {
  const next = { ...ch };
  if (patch.name !== undefined)
    next.name =
      ch.kind === "text" ? slugifyChannelName(patch.name) : patch.name.trim();
  if (patch.category_id !== undefined) next.category_id = patch.category_id;
  if (patch.role_ids !== undefined) {
    next.limited = patch.role_ids.length > 0;
    next.role_ids = patch.role_ids;
  }
  return next;
}

const sameIds = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((id, i) => id === b[i]);

/** A failed PATCH: each field it set goes back to `was`, unless something
 * later (another edit, an event) has replaced the value it wrote. */
function unpatchChannel(
  ch: Channel,
  was: Channel,
  patch: UpdateChannelInput,
): Channel {
  const ours = patchChannel(was, patch);
  const next = { ...ch };
  if (patch.name !== undefined && ch.name === ours.name) next.name = was.name;
  if (patch.category_id !== undefined && ch.category_id === ours.category_id)
    next.category_id = was.category_id;
  if (patch.role_ids !== undefined && sameIds(ch.role_ids, ours.role_ids)) {
    next.limited = was.limited;
    next.role_ids = was.role_ids;
  }
  return next;
}

export function updateChannelOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: ({ id, patch }: { id: string; patch: UpdateChannelInput }) =>
      remote.updateChannel(id, patch),
    onMutate: ({ id, patch }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          channels: detail.channels.map((ch) =>
            ch.id === id ? patchChannel(ch, patch) : ch,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, { id, patch }, ctx) => {
      if (!stampHolds(ctx)) return;
      const was = ctx.snap.detail?.channels.find((ch) => ch.id === id);
      if (was)
        patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          channels: detail.channels.map((ch) =>
            ch.id === id ? unpatchChannel(ch, was, patch) : ch,
          ),
        }));
      notifyError(error, translate(servers, "error.channelNotSaved"));
    },
    onSuccess: (channel, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        channels: detail.channels.map((ch) =>
          ch.id === channel.id ? channel : ch,
        ),
      }));
    },
  });
}

export function useUpdateChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation(updateChannelOptions(client, serverId));
}

export function deleteChannelOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: (id: string) => remote.deleteChannel(id),
    onMutate: (id) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          channels: detail.channels.filter((ch) => ch.id !== id),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, id, ctx) => {
      if (!stampHolds(ctx)) return;
      const before = ctx.snap.detail;
      const was = before?.channels.find((ch) => ch.id === id);
      if (before && was && !targetGone(error))
        patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          channels: putBack(detail.channels, before.channels, was, byId),
        }));
      notifyError(error, translate(servers, "error.channelNotDeleted"));
    },
  });
}

export function useDeleteChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation(deleteChannelOptions(client, serverId));
}

// ---------------------------------------------------------------------------
// Invites

export function useCreateInvite(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateInviteInput) =>
      remote.createInvite(serverId, input),
    onMutate: () => takeStamp(),
    onSuccess: (invite, _input, stamp) => {
      if (!stampHolds(stamp)) return;
      client.setQueryData<Invite[]>(
        serverKeys.invites(stamp.userId, stamp.generation, serverId),
        (current) => (current ? [invite, ...current] : current),
      );
    },
  });
}

export function useRevokeInvite(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (code: string) => remote.revokeInvite(code),
    onMutate: (code) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const previous = client.getQueryData<Invite[]>(
        serverKeys.invites(stamp.userId, stamp.generation, serverId),
      );
      client.setQueryData<Invite[]>(
        serverKeys.invites(stamp.userId, stamp.generation, serverId),
        (current) => current?.filter((invite) => invite.code !== code),
      );
      return { ...stamp, previous };
    },
    onError: (error, _code, ctx) => {
      if (!stampHolds(ctx)) return;
      if (ctx.previous) {
        client.setQueryData(
          serverKeys.invites(ctx.userId, ctx.generation, serverId),
          ctx.previous,
        );
      }
      notifyError(error, translate(servers, "error.inviteNotRevoked"));
    },
  });
}

function dropMember(
  client: QueryClient,
  ownerId: string,
  generation: number,
  serverId: string,
  memberId: string,
): void {
  patchDetail(client, ownerId, generation, serverId, (detail) => ({
    ...detail,
    members: detail.members.filter((member) => member.user_id !== memberId),
  }));
}

/** Another tab / WS: this user left or was removed. */
export function applyMemberRemoved(
  client: QueryClient,
  ownerId: string,
  generation: number,
  serverId: string,
  removedUserId: string,
): "self" | "other" {
  if (ownerId === removedUserId) {
    removeServer(client, ownerId, generation, serverId, { keepDetail: true });
    return "self";
  }
  dropMember(client, ownerId, generation, serverId, removedUserId);
  return "other";
}

/**
 * Someone joined (#621): they enter the cached member list at its end, in
 * join order like the server lists them. A repeat of the event (a replay
 * after a gap) updates their row instead of adding it twice.
 */
export function applyMemberJoined(
  client: QueryClient,
  ownerId: string,
  generation: number,
  serverId: string,
  member: Member,
): void {
  patchDetail(client, ownerId, generation, serverId, (detail) => {
    const at = detail.members.findIndex(
      (listed) => listed.user_id === member.user_id,
    );
    if (at < 0) return { ...detail, members: [...detail.members, member] };
    const members = [...detail.members];
    members[at] = { ...members[at], ...member };
    return { ...detail, members };
  });
}

/** A kick or ban that failed: the member goes back into the list, unless
 * they had left already. */
function undoDropMember(
  client: QueryClient,
  ctx: { userId: string; generation: number; snap: Snapshot },
  serverId: string,
  memberId: string,
  error: unknown,
): void {
  const before = ctx.snap.detail?.members;
  const was = before?.find((member) => member.user_id === memberId);
  if (!before || !was || targetGone(error)) return;
  patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
    ...detail,
    members: putBack(detail.members, before, was, (m) => m.user_id),
  }));
}

export function kickMemberOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: (memberId: string) => remote.kickMember(serverId, memberId),
    onMutate: (memberId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      dropMember(client, stamp.userId, stamp.generation, serverId, memberId);
      return { ...stamp, snap };
    },
    onError: (error, memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      undoDropMember(client, ctx, serverId, memberId, error);
      notifyError(error, translate(servers, "error.memberNotKicked"));
    },
  });
}

export function useKickMember(serverId: string) {
  const client = useQueryClient();
  return useMutation(kickMemberOptions(client, serverId));
}

export function banMemberOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: (memberId: string) => remote.banMember(serverId, memberId),
    onMutate: (memberId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      dropMember(client, stamp.userId, stamp.generation, serverId, memberId);
      return { ...stamp, snap };
    },
    onError: (error, memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      undoDropMember(client, ctx, serverId, memberId, error);
      notifyError(error, translate(servers, "error.memberNotBanned"));
    },
    onSuccess: (_data, _memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      void client.invalidateQueries({
        queryKey: serverKeys.bans(ctx.userId, ctx.generation, serverId),
      });
    },
  });
}

export function useBanMember(serverId: string) {
  const client = useQueryClient();
  return useMutation(banMemberOptions(client, serverId));
}

export function useUnbanMember(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (memberId: string) => remote.unbanMember(serverId, memberId),
    onMutate: (memberId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const previous = client.getQueryData<Ban[]>(
        serverKeys.bans(stamp.userId, stamp.generation, serverId),
      );
      client.setQueryData<Ban[]>(
        serverKeys.bans(stamp.userId, stamp.generation, serverId),
        (current) => current?.filter((ban) => ban.user_id !== memberId),
      );
      return { ...stamp, previous };
    },
    onError: (error, _memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      if (ctx.previous) {
        client.setQueryData(
          serverKeys.bans(ctx.userId, ctx.generation, serverId),
          ctx.previous,
        );
      }
      notifyError(error, translate(servers, "error.memberNotUnbanned"));
    },
  });
}

export function useJoinInvite() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (code: string) => remote.joinInvite(code),
    onMutate: () => takeStamp(),
    onSuccess: (server, _code, stamp) => {
      if (!stampHolds(stamp)) return;
      rememberJoinedServer(client, stamp, server);
    },
  });
}

/** A server just joined (by invite or from the directory) joins the list. */
export function rememberJoinedServer(
  client: QueryClient,
  stamp: { userId: string; generation: number },
  server: Server,
): void {
  const { userId, generation } = stamp;
  patchList(client, userId, generation, (servers) =>
    servers.some((s) => s.id === server.id) ? servers : [...servers, server],
  );
  // Members and channels are not part of the join answer.
  void client.invalidateQueries({
    queryKey: serverKeys.detail(userId, generation, server.id),
  });
}

// ---------------------------------------------------------------------------
// Roles

/** The signed-in user's flags in a cached detail, as the server computes
 * them; `undefined` when they are not among its members. */
function ownPermissions(
  detail: ServerDetail,
  userId: string,
): Permission[] | undefined {
  const me = detail.members.find((member) => member.user_id === userId);
  return me ? effectivePermissions(me, detail) : undefined;
}

/**
 * Applies a role change to the cached detail and recomputes the caller's
 * own flags from it, in the detail and in the list row, so a page gated on
 * a right reacts before the answer comes back.
 */
function patchRoles(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
  update: (detail: ServerDetail) => ServerDetail,
): void {
  let permissions: Permission[] | undefined;
  patchDetail(client, userId, generation, serverId, (current) => {
    const detail = update(current);
    permissions = ownPermissions(detail, userId);
    return permissions ? { ...detail, permissions } : detail;
  });
  if (permissions) {
    const mine = permissions;
    patchList(client, userId, generation, (servers) =>
      servers.map((s) => (s.id === serverId ? { ...s, permissions: mine } : s)),
    );
  }
}

function byPosition(a: ServerRole, b: ServerRole): number {
  return a.position - b.position || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** Role ids in the order of the server's roles, as the API returns them. */
function inRoleOrder(detail: ServerDetail, ids: readonly string[]): string[] {
  const wanted = new Set(ids);
  return [...detail.roles]
    .sort(byPosition)
    .map((role) => role.id)
    .filter((id) => wanted.has(id));
}

/**
 * Role writes of one server overlap the way `useUpdateServer`'s do (switches
 * and checkboxes clicked in a row): each is applied at once, only the last
 * one in flight writes its answer back, and a failure inside a burst ends it
 * with a refetch instead of a rollback that would wipe the later clicks.
 * Every role write of a server shares the key, so a rename, a delete and an
 * assignment never roll each other back either.
 */
const resyncRoles = new Set<string>();

function roleBurst(client: QueryClient, userId: string, serverId: string) {
  const mutationKey = ["servers", "roles", userId, serverId];
  const id = `${userId}/${serverId}`;
  const last = () => client.isMutating({ mutationKey }) <= 1;
  return {
    mutationKey,
    last,
    /** On error: true when the snapshot may be restored. */
    failed(): boolean {
      if (last() && !resyncRoles.has(id)) return true;
      resyncRoles.add(id);
      return false;
    },
    /** On settle: true when the burst ended after a failure. */
    settled(): boolean {
      if (!last() || !resyncRoles.has(id)) return false;
      resyncRoles.delete(id);
      return true;
    },
  };
}

function refetchServer(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): void {
  void client.invalidateQueries({
    queryKey: serverKeys.detail(userId, generation, serverId),
  });
  void client.invalidateQueries({
    queryKey: serverKeys.list(userId, generation),
  });
}

/** The caller's role and server writes of a server still in flight. */
function writesInFlight(
  client: QueryClient,
  userId: string,
  serverId: string,
): number {
  return (
    client.isMutating({ mutationKey: ["servers", "roles", userId, serverId] }) +
    client.isMutating({ mutationKey: ["servers", "update", userId, serverId] })
  );
}

const deferredAccess = new Set<string>();

/**
 * The access-changed event (`{"k":"a"}`) reads the server and the list
 * again. Our own writes announce it too, so during a burst the refetch
 * could land between two PATCHes and the next click would build on it;
 * it waits until the burst has settled.
 */
export function refetchOnAccessChange(
  client: QueryClient,
  userId: string,
  generation: number,
  serverId: string,
): void {
  if (writesInFlight(client, userId, serverId) > 0) {
    deferredAccess.add(`${userId}/${serverId}`);
    return;
  }
  refetchServer(client, userId, generation, serverId);
}

/** On settle of a burst write: true when an access change waited for the
 * last write of the server to end. */
function takeDeferredAccess(
  client: QueryClient,
  userId: string,
  serverId: string,
): boolean {
  const id = `${userId}/${serverId}`;
  if (!deferredAccess.has(id) || writesInFlight(client, userId, serverId) > 1)
    return false;
  deferredAccess.delete(id);
  return true;
}

/** The end of a burst write: refetch after a failure in the burst or an
 * access change that waited for it. */
function settleBurst(
  client: QueryClient,
  ctx: { userId: string; generation: number },
  serverId: string,
  failed: boolean,
): void {
  const deferred = takeDeferredAccess(client, ctx.userId, serverId);
  if (failed || deferred)
    refetchServer(client, ctx.userId, ctx.generation, serverId);
}

/** The caller's own flags may change with any role write. */
function refetchList(
  client: QueryClient,
  userId: string,
  generation: number,
): void {
  void client.invalidateQueries({
    queryKey: serverKeys.list(userId, generation),
  });
}

/** Adds a role after the others. It shows at once with a `tmp:` id that is
 * swapped for the real one; until then it cannot be edited or assigned. */
export function createRoleOptions(client: QueryClient, serverId: string) {
  return mutationOptions({
    mutationFn: (input: CreateRoleInput) => remote.createRole(serverId, input),
    onMutate: (input) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const tmp = tmpId();
      patchDetail(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          roles: [
            ...detail.roles,
            {
              id: tmp,
              server_id: serverId,
              name: input.name.trim(),
              color: input.color,
              permissions: input.permissions ?? [],
              position:
                Math.max(-1, ...detail.roles.map((role) => role.position)) + 1,
              created_at: now(),
            },
          ],
        }),
      );
      return { ...stamp, tmp };
    },
    onSuccess: (role, _input, ctx) => {
      if (!stampHolds(ctx)) return;
      patchRoles(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        roles: replaceRow(detail.roles, ctx.tmp, role),
      }));
      refetchList(client, ctx.userId, ctx.generation);
    },
    onError: (error, _input, ctx) => {
      if (!stampHolds(ctx)) return;
      patchDetail(client, ctx.userId, ctx.generation, serverId, (detail) => ({
        ...detail,
        roles: replaceRow(detail.roles, ctx.tmp, null),
      }));
      notifyError(error, translate(servers, "error.roleNotCreated"));
    },
  });
}

/** Renames, recolours or replaces the flags of a role. */
export function updateRoleOptions(
  client: QueryClient,
  userId: string,
  serverId: string,
) {
  const burst = roleBurst(client, userId, serverId);
  return mutationOptions({
    mutationKey: burst.mutationKey,
    mutationFn: ({ id, patch }: { id: string; patch: UpdateRoleInput }) =>
      remote.updateRole(id, patch),
    onMutate: ({ id, patch }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchRoles(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          roles: detail.roles.map((role) =>
            role.id === id
              ? {
                  ...role,
                  ...(patch.name !== undefined
                    ? { name: patch.name.trim() }
                    : {}),
                  ...(patch.color !== undefined ? { color: patch.color } : {}),
                  ...(patch.permissions !== undefined
                    ? { permissions: patch.permissions }
                    : {}),
                }
              : role,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      if (burst.failed())
        restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error, translate(servers, "error.roleNotSaved"));
    },
    onSuccess: (role, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      if (burst.last())
        patchRoles(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          roles: detail.roles.map((r) => (r.id === role.id ? role : r)),
        }));
      refetchList(client, ctx.userId, ctx.generation);
    },
    onSettled: (_data, _error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      settleBurst(client, ctx, serverId, burst.settled());
    },
  });
}

/** Members lose the role's flags; a channel limited to it stays limited. */
export function deleteRoleOptions(
  client: QueryClient,
  userId: string,
  serverId: string,
) {
  const burst = roleBurst(client, userId, serverId);
  return mutationOptions({
    mutationKey: burst.mutationKey,
    mutationFn: (id: string) => remote.deleteRole(id),
    onMutate: (id) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchRoles(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          roles: detail.roles.filter((role) => role.id !== id),
          members: detail.members.map((member) =>
            member.role_ids.includes(id)
              ? {
                  ...member,
                  role_ids: member.role_ids.filter((r) => r !== id),
                }
              : member,
          ),
          channels: detail.channels.map((channel) =>
            channel.role_ids.includes(id)
              ? {
                  ...channel,
                  role_ids: channel.role_ids.filter((r) => r !== id),
                }
              : channel,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      if (burst.failed())
        restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error, translate(servers, "error.roleNotDeleted"));
    },
    onSuccess: (_data, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      refetchList(client, ctx.userId, ctx.generation);
    },
    onSettled: (_data, _error, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      settleBurst(client, ctx, serverId, burst.settled());
    },
  });
}

/** Full replacement of a member's custom roles. */
export function setMemberRolesOptions(
  client: QueryClient,
  userId: string,
  serverId: string,
) {
  const burst = roleBurst(client, userId, serverId);
  return mutationOptions({
    mutationKey: burst.mutationKey,
    mutationFn: ({
      userId: memberId,
      roleIds,
    }: {
      userId: string;
      roleIds: string[];
    }) => remote.setMemberRoles(serverId, memberId, roleIds),
    onMutate: ({ userId: memberId, roleIds }) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      patchRoles(
        client,
        stamp.userId,
        stamp.generation,
        serverId,
        (detail) => ({
          ...detail,
          members: detail.members.map((member) =>
            member.user_id === memberId
              ? { ...member, role_ids: inRoleOrder(detail, roleIds) }
              : member,
          ),
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      if (burst.failed())
        restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error, translate(servers, "error.memberRolesNotSaved"));
    },
    onSuccess: (result, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      if (burst.last())
        patchRoles(client, ctx.userId, ctx.generation, serverId, (detail) => ({
          ...detail,
          members: detail.members.map((member) =>
            member.user_id === result.user_id
              ? { ...member, role_ids: result.role_ids }
              : member,
          ),
        }));
      refetchList(client, ctx.userId, ctx.generation);
    },
    onSettled: (_data, _error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      settleBurst(client, ctx, serverId, burst.settled());
    },
  });
}

export function useCreateRole(serverId: string) {
  const client = useQueryClient();
  return useMutation(createRoleOptions(client, serverId));
}

export function useUpdateRole(serverId: string) {
  const client = useQueryClient();
  return useMutation(updateRoleOptions(client, useUserId(), serverId));
}

export function useDeleteRole(serverId: string) {
  const client = useQueryClient();
  return useMutation(deleteRoleOptions(client, useUserId(), serverId));
}

export function useSetMemberRoles(serverId: string) {
  const client = useQueryClient();
  return useMutation(setMemberRolesOptions(client, useUserId(), serverId));
}

/**
 * Hands the server to another member (#519). Not optimistic: the answer is
 * the caller's new view of the server, and the members' roles and the
 * caller's rights are read again.
 */
export function useTransferServer(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (userId: string) => remote.transferServer(serverId, userId),
    onMutate: () => takeStamp(),
    onSuccess: (server, _userId, stamp) => {
      if (!stampHolds(stamp)) return;
      mergeServer(client, stamp.userId, stamp.generation, server);
      refetchServer(client, stamp.userId, stamp.generation, serverId);
    },
  });
}
