import { useQuery } from "@tanstack/react-query";

import { api } from "../api/client.ts";
import type { AuthProvider } from "./types.ts";

/**
 * The sign-in providers the instance offers (`GET /api/auth/providers`).
 * Fixed per deployment, so it is asked once; while unknown or after a
 * failed check the list is empty and the auth pages look as without any.
 */
export function useAuthProviders(): AuthProvider[] {
  const { data } = useQuery({
    queryKey: ["instance", "providers"],
    queryFn: ({ signal }) =>
      api<{ providers: AuthProvider[] }>("/auth/providers", { signal }),
    staleTime: Infinity,
    retry: false,
  });
  return data?.providers ?? NONE;
}

const NONE: AuthProvider[] = [];
