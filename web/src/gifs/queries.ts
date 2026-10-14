import { useInfiniteQuery, useQuery } from "@tanstack/react-query";

import { scopeGeneration, useUserId } from "../auth/scope.ts";
import { gifPage, gifStatus } from "./api.ts";

/** Whether the server relays GIFs. Fixed per deployment; a failed check
 *  hides the button rather than retrying. */
export function useGifStatus() {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: ["user", userId, generation, "gifs", "status"],
    queryFn: ({ signal }) => gifStatus(signal),
    enabled: userId.length > 0,
    staleTime: Infinity,
    retry: false,
  });
}

/** Trending for an empty query, else search. Pages in the provider's order. */
export function useGifFeed(query: string, enabled: boolean) {
  const userId = useUserId();
  const generation = scopeGeneration();
  const q = query.trim();
  return useInfiniteQuery({
    queryKey: ["user", userId, generation, "gifs", "feed", q],
    queryFn: ({ pageParam, signal }) => gifPage(q, pageParam, signal),
    initialPageParam: 1,
    getNextPageParam: (page) => page.next_page ?? undefined,
    enabled: enabled && userId.length > 0,
    staleTime: 60_000,
    retry: false,
  });
}
