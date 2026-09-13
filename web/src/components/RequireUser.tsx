import { useLocation } from "@tanstack/react-router";
import { useEffect, useRef, type ReactNode } from "react";

import { useSession } from "../auth/session.ts";
import { Redirect } from "./Redirect.tsx";

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

  // The post-login target is the last page seen *while signed in*. Once the
  // redirect starts, `location` already reflects `/login?redirect=…`; using
  // it live would nest the redirect into itself on every render.
  const target = useRef(location.href);
  useEffect(() => {
    if (status === "authenticated") target.current = location.href;
  }, [status, location.href]);

  if (status === "authenticated") {
    return children;
  }
  if (status === "unknown") {
    return null;
  }
  return (
    <Redirect
      to="/login"
      search={target.current === "/" ? {} : { redirect: target.current }}
    />
  );
}
