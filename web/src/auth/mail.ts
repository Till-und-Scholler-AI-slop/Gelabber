import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";

import { api } from "../api/client.ts";

/**
 * Whether the server sends account mail (confirmation and reset links).
 * Fixed per deployment, so it is asked once; while unknown or after a
 * failed check, the sign-in page simply offers no "forgot password".
 */
export function useAccountMail(): boolean {
  const { data } = useQuery({
    queryKey: ["instance", "mail"],
    queryFn: ({ signal }) =>
      api<{ enabled: boolean }>("/auth/mail", { signal }),
    staleTime: Infinity,
    retry: false,
  });
  return data?.enabled === true;
}

/**
 * The token of a mailed link (`…#token=<hex>`). The fragment never reaches
 * a server log.
 */
export function readLinkToken(hash: string): string {
  const match = /(?:^#|&)token=([0-9a-f]{64})(?:&|$)/i.exec(hash);
  return match?.[1]?.toLowerCase() ?? "";
}

/** Drops the token from the address bar, so the history does not keep it. */
export function forgetLinkToken(): void {
  if (typeof window === "undefined" || !window.location.hash) return;
  window.history.replaceState(
    window.history.state,
    "",
    window.location.pathname + window.location.search,
  );
}

/** The link token this page was opened with, read once. */
export function useLinkToken(): string {
  const [token] = useState(() =>
    typeof window === "undefined" ? "" : readLinkToken(window.location.hash),
  );
  useEffect(forgetLinkToken, []);
  return token;
}
