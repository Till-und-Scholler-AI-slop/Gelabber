// TanStack Query layer for blocks (#565). A block or unblock on any of the
// user's devices arrives as the account frame `bl`, which reads the list
// again; the DM list follows, since a DM says whether it is blocked.

import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from "@tanstack/react-query";
import { useEffect, useMemo } from "react";

import {
  scopeGeneration,
  stampHolds,
  takeStamp,
  useUserId,
} from "../auth/scope.ts";
import { confirmAction } from "../components/confirm.ts";
import { notifyError } from "../components/toasts.ts";
import { dmKeys } from "../dms/queries.ts";
import { friendKeys } from "../friends/queries.ts";
import { translate } from "../i18n/catalog.ts";
import { blocks as blocksCatalog } from "../i18n/catalogs/blocks.ts";
import { getGateway } from "../ws/client.ts";
import type { AccountFrame } from "../ws/protocol.ts";
import * as remote from "./api.ts";
import type { BlockedUser } from "./types.ts";

export const blockKeys = {
  list: (userId: string, generation: number) =>
    ["user", userId, generation, "blocks"] as const,
};

export function useBlocks() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: blockKeys.list(userId, generation),
    queryFn: ({ signal }) => remote.listBlocks(signal),
    enabled: userId.length > 0,
    staleTime: Infinity,
  });
}

/**
 * Whether `authorId` is someone the user blocks, from what is cached: for
 * code outside React, such as the notification listener. False until the
 * list is in.
 */
export function blocksAuthor(
  client: QueryClient,
  userId: string,
  generation: number,
  authorId: string,
): boolean {
  return (
    client
      .getQueryData<BlockedUser[]>(blockKeys.list(userId, generation))
      ?.some((row) => row.id === authorId) ?? false
  );
}

/** The ids the signed-in user blocks; empty until the list is in. */
export function useBlockedIds(): ReadonlySet<string> {
  const { data } = useBlocks();
  return useMemo(() => new Set((data ?? []).map((b) => b.id)), [data]);
}

function refresh(client: QueryClient, userId: string): Promise<void> {
  const generation = scopeGeneration();
  return Promise.all([
    client.invalidateQueries({ queryKey: blockKeys.list(userId, generation) }),
    client.invalidateQueries({ queryKey: dmKeys.all(userId, generation) }),
    client.invalidateQueries({
      queryKey: friendKeys.list(userId, generation),
    }),
    client.invalidateQueries({
      queryKey: friendKeys.requests(userId, generation),
    }),
  ]).then(() => undefined);
}

function useBlockAction(
  run: (userId: string) => Promise<void>,
  failed: "block.failed" | "unblock.failed",
) {
  const client = useQueryClient();
  return useMutation({
    mutationFn: run,
    onMutate: () => takeStamp(),
    onSettled: (_data, error, _id, stamp) => {
      if (!stamp || !stampHolds(stamp)) return;
      if (error) notifyError(error, translate(blocksCatalog, failed));
      void refresh(client, stamp.userId);
    },
  });
}

export const useBlock = () => useBlockAction(remote.block, "block.failed");
export const useUnblock = () =>
  useBlockAction(remote.unblock, "unblock.failed");

/** "{name} blockieren?" with what it does; resolves to the answer. */
export function confirmBlock(name: string): Promise<boolean> {
  return confirmAction({
    title: translate(blocksCatalog, "confirm.title", { name }),
    message: translate(blocksCatalog, "confirm.message"),
    confirmLabel: translate(blocksCatalog, "action.block"),
    tone: "danger",
  });
}

/** What `bl` changes in the cache; other frames pass by. */
export function applyBlockFrame(
  client: QueryClient,
  userId: string,
  frame: AccountFrame,
): void {
  if (frame.op === "bl") void refresh(client, userId);
}

/** Mounted once in the app shell. */
export function useBlocksBridge(): void {
  const client = useQueryClient();
  const userId = useUserId();
  useEffect(() => {
    if (!userId) return;
    return getGateway().onAccount((frame) =>
      applyBlockFrame(client, userId, frame),
    );
  }, [client, userId]);
}
