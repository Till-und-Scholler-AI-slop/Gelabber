// TanStack Query layer for friends, friend codes and message requests
// (#563). Every change on any device arrives as an account frame: `fr`
// reloads the friend list, `mr` the incoming message requests (and the DM
// list, which an accepted request joins).

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useEffect } from "react";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { notifyError } from "../components/toasts.ts";
import * as dmRemote from "../dms/api.ts";
import { cacheOpenedDm, dmKeys } from "../dms/queries.ts";
import type { DirectMessage } from "../dms/types.ts";
import { translate } from "../i18n/catalog.ts";
import { friends as friendsCatalog } from "../i18n/catalogs/friends.ts";
import type { AccountFrame } from "../ws/protocol.ts";
import { getGateway } from "../ws/client.ts";
import * as remote from "./api.ts";
import type { FriendList, Relation } from "./types.ts";

export const friendKeys = {
  list: (userId: string, generation: number) =>
    ["user", userId, generation, "friends", "list"] as const,
  code: (userId: string, generation: number) =>
    ["user", userId, generation, "friends", "code"] as const,
  requests: (userId: string, generation: number) =>
    ["user", userId, generation, "dms", "requests"] as const,
};

const STALE_MS = 60_000;
/** Online dots come with the list, not live: read again this often. */
const STATUS_REFRESH_MS = 60_000;

export function useFriends(options: { live?: boolean } = {}) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: friendKeys.list(userId, generation),
    queryFn: ({ signal }) => remote.listFriends(signal),
    enabled: userId.length > 0,
    staleTime: STALE_MS,
    refetchInterval: options.live ? STATUS_REFRESH_MS : false,
  });
}

export function useFriendCode() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: friendKeys.code(userId, generation),
    queryFn: ({ signal }) => remote.getFriendCode(signal),
    enabled: userId.length > 0,
    staleTime: Infinity,
  });
}

export function useDmRequests() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: friendKeys.requests(userId, generation),
    queryFn: ({ signal }) => dmRemote.listRequests(signal),
    enabled: userId.length > 0,
    staleTime: STALE_MS,
  });
}

/** How the signed-in user stands to `userId`, from the cached list. */
export function relationTo(
  list: FriendList | undefined,
  me: string,
  userId: string,
): Relation | undefined {
  if (userId === me) return "self";
  if (!list) return undefined;
  if (list.friends.some((f) => f.user.id === userId)) return "friend";
  if (list.outgoing.some((r) => r.user.id === userId)) return "outgoing";
  if (list.incoming.some((r) => r.user.id === userId)) return "incoming";
  return "none";
}

function refreshFriends(client: QueryClient, userId: string): Promise<void> {
  return client.invalidateQueries({
    queryKey: friendKeys.list(userId, scopeGeneration()),
  });
}

function refreshRequests(client: QueryClient, userId: string): Promise<void> {
  const generation = scopeGeneration();
  return Promise.all([
    client.invalidateQueries({
      queryKey: friendKeys.requests(userId, generation),
    }),
    client.invalidateQueries({ queryKey: dmKeys.all(userId, generation) }),
  ]).then(() => undefined);
}

/** One mutation shape for every friend action: reload, or say why not. */
function useFriendAction<A, R>(
  run: (arg: A) => Promise<R>,
  failed: "send.failed" | "action.failed",
) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: run,
    onMutate: () => takeStamp(),
    onSuccess: (_data, _arg, stamp) => {
      if (stampHolds(stamp)) void refreshFriends(client, stamp.userId);
    },
    onError: (error, _arg, stamp) => {
      if (!stampHolds(stamp)) return;
      notifyError(error, translate(friendsCatalog, failed));
      void refreshFriends(client, stamp.userId);
    },
  });
}

export const useSendFriendRequest = () =>
  useFriendAction(remote.sendRequest, "send.failed");
export const useAcceptFriend = () =>
  useFriendAction(remote.acceptRequest, "action.failed");
export const useDeclineFriend = () =>
  useFriendAction(remote.declineRequest, "action.failed");
export const useWithdrawFriend = () =>
  useFriendAction(remote.withdrawRequest, "action.failed");
export const useRemoveFriend = () =>
  useFriendAction(remote.removeFriend, "action.failed");

export function useRegenerateFriendCode() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: remote.regenerateFriendCode,
    onMutate: () => takeStamp(),
    onSuccess: (code, _arg, stamp) => {
      if (!stampHolds(stamp)) return;
      client.setQueryData(
        friendKeys.code(stamp.userId, stamp.generation),
        code,
      );
    },
    onError: (error, _arg, stamp) => {
      if (stampHolds(stamp))
        notifyError(error, translate(friendsCatalog, "code.regenerateFailed"));
    },
  });
}

/** Accept a message request: it becomes an ordinary DM in the list. */
export function useAcceptDmRequest() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (channelId: string) => dmRemote.acceptRequest(channelId),
    onMutate: () => takeStamp(),
    onSuccess: (dm: DirectMessage, _id, stamp) => {
      if (!stampHolds(stamp)) return;
      cacheOpenedDm(client, stamp, dm);
      void refreshRequests(client, stamp.userId);
    },
    onError: (error, _id, stamp) => {
      if (stampHolds(stamp))
        notifyError(error, translate(friendsCatalog, "action.failed"));
    },
  });
}

/** Decline quietly: the request leaves the tab; the sender is not told. */
export function useDeclineDmRequest() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (channelId: string) => dmRemote.declineRequest(channelId),
    onMutate: () => takeStamp(),
    onSuccess: (_data, channelId, stamp) => {
      if (!stampHolds(stamp)) return;
      client.setQueryData<DirectMessage[]>(
        friendKeys.requests(stamp.userId, stamp.generation),
        (current) => current?.filter((dm) => dm.id !== channelId),
      );
      void refreshRequests(client, stamp.userId);
    },
    onError: (error, _id, stamp) => {
      if (stampHolds(stamp))
        notifyError(error, translate(friendsCatalog, "action.failed"));
    },
  });
}

/** What `fr` and `mr` change in the cache; other frames pass by. */
export function applyFriendFrame(
  client: QueryClient,
  userId: string,
  frame: AccountFrame,
): void {
  if (frame.op === "fr") void refreshFriends(client, userId);
  if (frame.op === "mr") void refreshRequests(client, userId);
}

/** Mounted once in the app shell. */
export function useFriendsBridge(): void {
  const client = useQueryClient();
  const userId = useUserId();
  useEffect(() => {
    if (!userId) return;
    return getGateway().onAccount((frame) =>
      applyFriendFrame(client, userId, frame),
    );
  }, [client, userId]);
}
