import { useLocation } from "@tanstack/react-router";
import { useState, type ReactNode } from "react";

import {
  loggedOutOnPurpose,
  useSession,
  useSessionProblems,
} from "../auth/session.ts";
import { AppPlaceholder } from "../pages/AuthLayout.tsx";
import { Redirect } from "./Redirect.tsx";

/**
 * Renders `children` only while the store holds a user. The route guard
 * already handles the first load; this covers a session that ends *while*
 * the page is open (401 from a mutation, logout in another tab) — the
 * store flips to anonymous and the page bounces to /login right away,
 * keeping the current URL as the post-login target and saying why.
 */
export function RequireUser({ children }: { children: ReactNode }) {
  const status = useSession((state) => state.status);
  const checkFailed = useSessionProblems((state) => state.bootstrap !== null);
  const location = useLocation();

  // The post-login target is the last page seen *while signed in*. Once the
  // redirect starts, `location` already reflects `/login?redirect=…`; using
  // it live would nest the redirect into itself on every render.
  const [target, setTarget] = useState(location.href);
  const [wasSignedIn, setWasSignedIn] = useState(status === "authenticated");
  if (status === "authenticated" && target !== location.href) {
    setTarget(location.href);
  }
  if (status === "authenticated" && !wasSignedIn) {
    setWasSignedIn(true);
  }

  if (status === "authenticated") {
    return children;
  }
  if (status === "unknown") {
    // A failed check is shown by SessionRecoveryNotice instead.
    return checkFailed ? null : <AppPlaceholder />;
  }
  return (
    <Redirect
      to="/login"
      search={{
        ...(target === "/" ? {} : { redirect: target }),
        ...(wasSignedIn && !loggedOutOnPurpose()
          ? { reason: "expired" as const }
          : {}),
      }}
    />
  );
}
