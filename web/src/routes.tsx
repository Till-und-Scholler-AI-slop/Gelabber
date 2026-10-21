import {
  createRootRoute,
  createRoute,
  createRouter,
  notFound,
  redirect,
} from "@tanstack/react-router";

import { safeRedirect } from "./auth/redirect.ts";
import { currentUserId } from "./auth/scope.ts";
import {
  ensureSession,
  useSession,
  useSessionProblems,
} from "./auth/session.ts";
import { AppShell } from "./components/AppShell.tsx";
import { RequireUser } from "./components/RequireUser.tsx";
import { ChannelPage } from "./pages/ChannelPage.tsx";
import { DmChannelPage } from "./pages/DmChannelPage.tsx";
import { DmIndexPage } from "./pages/DmIndexPage.tsx";
import { AppPlaceholder, AuthLayout } from "./pages/AuthLayout.tsx";
import { ForgotPasswordPage } from "./pages/ForgotPasswordPage.tsx";
import { InvitePage } from "./pages/InvitePage.tsx";
import { LoginPage } from "./pages/LoginPage.tsx";
import { NotFoundCard, NotFoundPage } from "./pages/NotFoundPage.tsx";
import { RegisterPage } from "./pages/RegisterPage.tsx";
import { ResetPasswordPage } from "./pages/ResetPasswordPage.tsx";
import { ServerPage } from "./pages/ServerPage.tsx";
import { ServerSettingsPage } from "./pages/ServerSettingsPage.tsx";
import {
  DEFAULT_SETTINGS_AREA,
  isSettingsArea,
  settingsListFirst,
} from "./pages/settingsAreas.ts";
import {
  SettingsListPage,
  UserSettingsPage,
} from "./pages/UserSettingsPage.tsx";
import { VerifyEmailPage } from "./pages/VerifyEmailPage.tsx";
import { WorkspaceIndexPage } from "./pages/WorkspaceIndexPage.tsx";
import { WorkspaceLayout } from "./pages/WorkspaceLayout.tsx";
import { placeFromPath, useLastPlace } from "./servers/lastPlace.ts";

const rootRoute = createRootRoute({
  component: AppShell,
});

export type AuthSearch = {
  redirect?: string;
  /** "expired": the session ended while the app was open. */
  reason?: "expired";
  /** Carried from sign-in to "Passwort vergessen", so it is not retyped. */
  email?: string;
};

export const validateAuthSearch = (
  search: Record<string, unknown>,
): AuthSearch => {
  const out: AuthSearch = {};
  const target = safeRedirect(search.redirect);
  if (target) out.redirect = target;
  if (search.reason === "expired") out.reason = "expired";
  if (
    typeof search.email === "string" &&
    search.email.length > 0 &&
    search.email.length <= 254
  )
    out.email = search.email;
  return out;
};

/**
 * Guard for signed-in pages: waits for the (single) session bootstrap.
 * Anonymous visitors go to `signIn` and come back here afterwards.
 */
function requireUser(signIn: "/login" | "/register" = "/login") {
  return async ({ location }: { location: { href: string } }) => {
    await ensureSession();
    if (useSession.getState().status === "unknown") return;
    if (useSession.getState().status !== "authenticated") {
      throw redirect({
        to: signIn,
        search: location.href === "/" ? {} : { redirect: location.href },
        replace: true,
      });
    }
  };
}

/** Guard for login/register: a signed-in user goes straight in. */
async function requireAnonymous({ search }: { search: AuthSearch }) {
  await ensureSession();
  if (useSession.getState().status === "authenticated") {
    throw redirect({ href: search.redirect ?? "/", replace: true });
  }
}

// The profile became the first settings area; old links and bookmarks land
// there (the workspace guard sends signed-out visitors to the login first).
const profileRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/profile",
  beforeLoad: () => {
    throw redirect({
      to: "/settings/$area",
      params: { area: "profil" },
      replace: true,
    });
  },
});

// ---------------------------------------------------------------------------
// The front door: sign-in, mail links and the invite landing, without the
// app header (`bare`).

const authLayout = createRoute({
  getParentRoute: () => rootRoute,
  id: "auth",
  staticData: { bare: true },
  component: AuthLayout,
  notFoundComponent: NotFoundCard,
});

const loginRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/login",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: LoginPage,
});

const registerRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/register",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: RegisterPage,
});

const forgotPasswordRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/forgot-password",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: ForgotPasswordPage,
});

// Mailed links work signed in or not: either ends with this tab signed in
// to the account the link belongs to.
const resetPasswordRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/reset-password",
  beforeLoad: () => ensureSession(),
  component: ResetPasswordPage,
});

const verifyEmailRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/verify-email",
  beforeLoad: () => ensureSession(),
  component: VerifyEmailPage,
});

const inviteRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/invite/$code",
  // Most people opening an invite have no account yet.
  beforeLoad: requireUser("/register"),
  component: () => (
    <RequireUser>
      <InvitePage />
    </RequireUser>
  ),
});

// ---------------------------------------------------------------------------
// Workspace: server rail + channel sidebar + page.

const workspaceLayout = createRoute({
  getParentRoute: () => rootRoute,
  id: "workspace",
  beforeLoad: requireUser(),
  component: WorkspaceLayout,
  // The workspace <main> has no padding of its own.
  notFoundComponent: () => (
    <div className="px-6 py-10">
      <NotFoundCard />
    </div>
  ),
});

const indexRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/",
  component: WorkspaceIndexPage,
});

const serverRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/s/$serverId",
  component: ServerPage,
});

const channelRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/s/$serverId/c/$channelId",
  component: ChannelPage,
});

const serverSettingsRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/s/$serverId/settings",
  component: ServerSettingsPage,
});

// Personal settings live in the workspace too: the rail stays, and the
// sidebar becomes the list of settings areas.
const userSettingsRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/settings",
  beforeLoad: () => {
    // Phones get the list of areas; wider screens show it in the sidebar.
    if (!settingsListFirst())
      throw redirect({
        to: "/settings/$area",
        params: { area: DEFAULT_SETTINGS_AREA },
        replace: true,
      });
  },
  component: SettingsListPage,
});

const userSettingsAreaRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/settings/$area",
  beforeLoad: ({ params }) => {
    if (!isSettingsArea(params.area)) throw notFound();
  },
  component: UserSettingsPage,
});

const dmIndexRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/d",
  component: DmIndexPage,
});

const dmRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/d/$channelId",
  component: DmChannelPage,
});

const routeTree = rootRoute.addChildren([
  profileRoute,
  authLayout.addChildren([
    loginRoute,
    registerRoute,
    forgotPasswordRoute,
    resetPasswordRoute,
    verifyEmailRoute,
    inviteRoute,
  ]),
  workspaceLayout.addChildren([
    indexRoute,
    serverRoute,
    channelRoute,
    serverSettingsRoute,
    userSettingsRoute,
    userSettingsAreaRoute,
    dmIndexRoute,
    dmRoute,
  ]),
]);

export const router = createRouter({
  routeTree,
  // Only the very first navigation waits on the bootstrap request. The
  // placeholder carries on the static splash, and fades in elsewhere.
  defaultPendingComponent: AppPlaceholder,
  defaultPendingMs: 0,
  // The router keeps a shown pending component for 500 ms by default, which
  // left every cold start blank long after the bootstrap request finished.
  defaultPendingMinMs: 0,
  defaultNotFoundComponent: NotFoundPage,
});

// "/" reopens the server or DM the user was in last.
router.subscribe("onResolved", ({ toLocation }) => {
  const userId = currentUserId();
  const place = placeFromPath(toLocation.pathname);
  if (userId && place) useLastPlace.getState().remember(userId, place);
});

// A session check that succeeds after failing runs the guards again, so a
// page shown while the identity was unknown is checked against it.
useSessionProblems.subscribe((now, before) => {
  if (before.bootstrap !== null && now.bootstrap === null)
    void router.invalidate();
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
  interface StaticDataRouteOption {
    /** A front-door page: no app header above it. */
    bare?: boolean;
  }
}
