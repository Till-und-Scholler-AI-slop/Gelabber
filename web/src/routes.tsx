import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { ensureSession, useSession } from "./auth/session.ts";
import { AppShell } from "./components/AppShell.tsx";
import { HomePage } from "./pages/HomePage.tsx";
import { LoginPage } from "./pages/LoginPage.tsx";
import { ProfilePage } from "./pages/ProfilePage.tsx";
import { RegisterPage } from "./pages/RegisterPage.tsx";

const rootRoute = createRootRoute({
  component: AppShell,
});

/** Only same-app paths may be used as post-login targets. */
function safeRedirect(value: unknown): string | undefined {
  return typeof value === "string" &&
    value.startsWith("/") &&
    !value.startsWith("//")
    ? value
    : undefined;
}

type AuthSearch = { redirect?: string };

const validateAuthSearch = (search: Record<string, unknown>): AuthSearch => {
  const target = safeRedirect(search.redirect);
  return target ? { redirect: target } : {};
};

/** Guard for signed-in pages: waits for the (single) session bootstrap. */
async function requireUser({ location }: { location: { href: string } }) {
  await ensureSession();
  if (useSession.getState().status !== "authenticated") {
    throw redirect({
      to: "/login",
      search: location.href === "/" ? {} : { redirect: location.href },
      replace: true,
    });
  }
}

/** Guard for login/register: a signed-in user goes straight in. */
async function requireAnonymous({ search }: { search: AuthSearch }) {
  await ensureSession();
  if (useSession.getState().status === "authenticated") {
    throw redirect({ href: search.redirect ?? "/", replace: true });
  }
}

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: requireUser,
  component: HomePage,
});

const profileRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/profile",
  beforeLoad: requireUser,
  component: ProfilePage,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: LoginPage,
});

const registerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/register",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: RegisterPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  profileRoute,
  loginRoute,
  registerRoute,
]);

export const router = createRouter({
  routeTree,
  // Only the very first navigation waits on the bootstrap request; render
  // nothing rather than a spinner for that split second.
  defaultPendingComponent: () => null,
  defaultPendingMs: 0,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
