import {
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
} from "@tanstack/react-router";

import { ensureSession, useSession } from "./auth/session.ts";
import { AppShell } from "./components/AppShell.tsx";
import { RequireUser } from "./components/RequireUser.tsx";
import { ChannelPage } from "./pages/ChannelPage.tsx";
import { DmChannelPage } from "./pages/DmChannelPage.tsx";
import { DmIndexPage } from "./pages/DmIndexPage.tsx";
import { InvitePage } from "./pages/InvitePage.tsx";
import { LoginPage } from "./pages/LoginPage.tsx";
import { ProfilePage } from "./pages/ProfilePage.tsx";
import { RegisterPage } from "./pages/RegisterPage.tsx";
import { ServerPage } from "./pages/ServerPage.tsx";
import { ServerSettingsPage } from "./pages/ServerSettingsPage.tsx";
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
// Centered pages: auth, profile, invite landing.

const centeredLayout = createRoute({
  getParentRoute: () => rootRoute,
  id: "centered",
  component: () => (
    <main className="mx-auto max-w-3xl px-6 py-10">
      <Outlet />
    </main>
  ),
});

const profileRoute = createRoute({
  getParentRoute: () => centeredLayout,
  path: "/profile",
  beforeLoad: requireUser,
  component: () => (
    <RequireUser>
      <ProfilePage />
    </RequireUser>
  ),
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
  centeredLayout.addChildren([
    profileRoute,
    loginRoute,
    registerRoute,
    inviteRoute,
  ]),
  workspaceLayout.addChildren([
    indexRoute,
    serverRoute,
    channelRoute,
    serverSettingsRoute,
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
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
