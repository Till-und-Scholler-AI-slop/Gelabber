import { Navigate, useLocation } from "@tanstack/react-router";
import type { ReactNode } from "react";

import { useSession } from "../auth/session.ts";

/**
 * Renders `children` only while the store holds a user. The route guard
 * already handles the first load; this covers a session that ends *while*
 * the page is open (401 from a mutation, logout in another tab) — the
 * store flips to anonymous and the page bounces to /login right away,
 * keeping the current URL as the post-login target.
 */
export function RequireUser({ children }: { children: ReactNode }) {
  const status = useSession((state) => state.status);
  const location = useLocation();

  if (status === "authenticated") {
    return children;
  }
  if (status === "unknown") {
    return null;
  }
  return (
    <Navigate
      to="/login"
      search={location.href === "/" ? {} : { redirect: location.href }}
      replace
    />
  );
}
