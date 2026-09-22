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
import { useRef } from "react";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import * as remote from "./api.ts";
import { slugifyChannelName } from "./rules.ts";
import type {
  Ban,
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
  return `${TMP_PREFIX}${crypto.randomUUID()}`;
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
        (error.code === "not_found" || error.code === "forbidden")
      ),
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
  options: { keepDetail?: boolean } = {},
): void {
  patchList(client, userId, generation, (servers) =>
    servers.filter((s) => s.id !== serverId),
  );
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
      notifyError(error);
    },
    onSuccess: (server, _patch, ctx) => {
      if (!stampHolds(ctx)) return;
      if (lastInFlight())
        mergeServer(client, ctx.userId, ctx.generation, server);
    },
    onSettled: (_data, _error, _patch, ctx) => {
      if (!stampHolds(ctx)) return;
      if (!lastInFlight() || !resync.current) return;
      resync.current = false;
      void client.invalidateQueries({
        queryKey: serverKeys.detail(ctx.userId, ctx.generation, serverId),
      });
      void client.invalidateQueries({
        queryKey: serverKeys.list(ctx.userId, ctx.generation),
      });
    },
  });
}

export function useDeleteServer() {
  const client = useQueryClient();
  return useMutation({
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
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error);
    },
  });
}

export function useLeaveServer() {
  const client = useQueryClient();
  return useMutation({
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
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
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
    onError: (error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error);
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

export function useDeleteCategory(serverId: string) {
  const client = useQueryClient();
  return useMutation({
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
    onError: (error, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
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
        }),
      );
      return { ...stamp, snap };
    },
    onError: (error, _vars, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error);
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

export function useDeleteChannel(serverId: string) {
  const client = useQueryClient();
  return useMutation({
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
    onError: (error, _id, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
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
      notifyError(error);
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

export function useKickMember(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (memberId: string) => remote.kickMember(serverId, memberId),
    onMutate: (memberId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      dropMember(client, stamp.userId, stamp.generation, serverId, memberId);
      return { ...stamp, snap };
    },
    onError: (error, _memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error);
    },
  });
}

export function useBanMember(serverId: string) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (memberId: string) => remote.banMember(serverId, memberId),
    onMutate: (memberId) => {
      const stamp = takeStamp();
      if (!stamp) return;
      const snap = snapshot(client, stamp.userId, stamp.generation, serverId);
      dropMember(client, stamp.userId, stamp.generation, serverId, memberId);
      return { ...stamp, snap };
    },
    onError: (error, _memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      restore(client, ctx.userId, ctx.generation, serverId, ctx.snap);
      notifyError(error);
    },
    onSuccess: (_data, _memberId, ctx) => {
      if (!stampHolds(ctx)) return;
      void client.invalidateQueries({
        queryKey: serverKeys.bans(ctx.userId, ctx.generation, serverId),
      });
    },
  });
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
      notifyError(error);
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
      const userId = stamp.userId;
      const generation = stamp.generation;
      patchList(client, userId, generation, (servers) =>
        servers.some((s) => s.id === server.id)
          ? servers
          : [...servers, server],
      );
      // Members and channels are not part of the join answer.
      void client.invalidateQueries({
        queryKey: serverKeys.detail(userId, generation, server.id),
      });
    },
  });
}
