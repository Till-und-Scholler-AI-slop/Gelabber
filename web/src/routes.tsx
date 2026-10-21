import {
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
  notFound,
  redirect,
} from "@tanstack/react-router";

import { ensureSession, useSession } from "./auth/session.ts";
import { AppShell } from "./components/AppShell.tsx";
import { RequireUser } from "./components/RequireUser.tsx";
import { ChannelPage } from "./pages/ChannelPage.tsx";
import { DmChannelPage } from "./pages/DmChannelPage.tsx";
import { DmIndexPage } from "./pages/DmIndexPage.tsx";
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
  if (useSession.getState().status === "unknown") return;
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

// ---------------------------------------------------------------------------
// Centered pages: auth and invite landing.

const centeredLayout = createRoute({
  getParentRoute: () => rootRoute,
  id: "centered",
  component: () => (
    <main className="mx-auto max-w-3xl px-6 py-10">
      <Outlet />
    </main>
  ),
  notFoundComponent: NotFoundCard,
});

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

const loginRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/login",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: LoginPage,
});

const registerRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/register",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: RegisterPage,
});

const forgotPasswordRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/forgot-password",
  validateSearch: validateAuthSearch,
  beforeLoad: requireAnonymous,
  component: ForgotPasswordPage,
});

// Mailed links work signed in or not: either ends with this tab signed in
// to the account the link belongs to.
const resetPasswordRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/reset-password",
  beforeLoad: () => ensureSession(),
  component: ResetPasswordPage,
});

const verifyEmailRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/verify-email",
  beforeLoad: () => ensureSession(),
  component: VerifyEmailPage,
});

const inviteRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/invite/$code",
  beforeLoad: requireUser,
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
  beforeLoad: requireUser,
  component: WorkspaceLayout,
  // The workspace <main> has no padding of its own (the centered one does).
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
  centeredLayout.addChildren([
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
  // Only the very first navigation waits on the bootstrap request; render
  // nothing rather than a spinner for that split second.
  defaultPendingComponent: () => null,
  defaultPendingMs: 0,
  // The router keeps a shown pending component for 500 ms by default, which
  // left every cold start blank long after the bootstrap request finished.
  defaultPendingMinMs: 0,
  defaultNotFoundComponent: NotFoundPage,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
