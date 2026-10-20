// Someone else's profile card (`GET /api/users/{id}`). Loaded when a card
// opens; the member list only carries its short parts.
//
// Keys include the signed-in user, like every other per-user query, so a
// later login never shows a card fetched for the previous account.

import { useQuery } from "@tanstack/react-query";

import { api } from "../api/client.ts";
import { scopeGeneration, useUserId } from "../auth/scope.ts";
import type { PublicProfile } from "../auth/types.ts";

export const profileKeys = {
  detail: (userId: string, generation: number, id: string) =>
    ["user", userId, generation, "profile", id] as const,
};

/** Short: an opened card should show what the person set a moment ago. */
const STALE_MS = 15_000;

export const getProfile = (id: string, signal?: AbortSignal) =>
  api<PublicProfile>(`/users/${encodeURIComponent(id)}`, { signal });

export function useUserProfile(id: string | null, enabled = true) {
  const userId = useUserId();
  const generation = scopeGeneration();
  return useQuery({
    queryKey: profileKeys.detail(userId, generation, id ?? ""),
    queryFn: ({ signal }) => getProfile(id ?? "", signal),
    enabled: userId.length > 0 && Boolean(id) && enabled,
    staleTime: STALE_MS,
    retry: (count, error) =>
      count < 2 &&
      !(
        error instanceof Error &&
        "code" in error &&
        error.code === "not_found"
      ),
  });
}
