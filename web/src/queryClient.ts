// One QueryClient for the life of the tab. User-owned entries are keyed
// `["user", userId, generation, …]` and dropped when that identity is no
// longer current, so a late response from the previous session cannot
// refill the next one — including the same person signing in again.

import { QueryClient, type Query } from "@tanstack/react-query";

export const USER_SCOPE = "user";

export const queryClient = new QueryClient({
  defaultOptions: {
    mutations: { retry: false },
  },
});

let activeUserId: string | null = null;
let activeGeneration = 0;

export function queryScopeUser(): string | null {
  return activeUserId;
}

export function setQueryScope(userId: string | null, generation: number): void {
  activeUserId = userId;
  activeGeneration = generation;
}

function isForeignUserQuery(query: Query): boolean {
  const key = query.queryKey;
  if (key[0] !== USER_SCOPE) return false;
  return key[1] !== activeUserId || key[2] !== activeGeneration;
}

/** Cancel in-flight reads and delete cache entries that are not the active user. */
export function dropForeignUserQueries(): void {
  void queryClient.cancelQueries({ predicate: isForeignUserQuery });
  queryClient.removeQueries({ predicate: isForeignUserQuery });
}

queryClient.getQueryCache().subscribe((event) => {
  if (event.type !== "added" && event.type !== "updated") return;
  const key = event.query.queryKey;
  if (key[0] !== USER_SCOPE) return;
  if (key[1] === activeUserId && key[2] === activeGeneration) return;
  const queryKey = event.query.queryKey;
  queueMicrotask(() => {
    if (queryKey[1] === activeUserId && queryKey[2] === activeGeneration) {
      return;
    }
    queryClient.removeQueries({ queryKey, exact: true });
  });
});
