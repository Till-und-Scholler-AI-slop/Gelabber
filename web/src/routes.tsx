import {
  createRootRoute,
  createRoute,
  createRouter,
  notFound,
  redirect,
} from "@tanstack/react-router";

import { ApiError } from "./api/client.ts";
import { safeRedirect } from "./auth/redirect.ts";
import { isOidcError, type OidcError } from "./auth/rules.ts";
import { currentUserId, takeStamp } from "./auth/scope.ts";
import { SECURITY_PATH } from "./auth/signInMethods.ts";
import {
  ensureSession,
  useSession,
  useSessionProblems,
} from "./auth/session.ts";
import { AppShell } from "./components/AppShell.tsx";
import { lazyPart } from "./components/lazyPart.tsx";
import { RequireUser } from "./components/RequireUser.tsx";
import { MainSkeleton } from "./components/ui/Skeleton.tsx";
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
import { RouteErrorPage } from "./pages/RouteErrorPage.tsx";
import { ServerPage } from "./pages/ServerPage.tsx";
import { SignInNamePage } from "./pages/SignInNamePage.tsx";
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
import { DiscoverPage } from "./pages/DiscoverPage.tsx";
import { ensureDirectoryStatus } from "./discover/queries.ts";
import { ensureFeedbackStatus } from "./feedback/queries.ts";
import { validateDiscoverSearch } from "./discover/search.ts";
import { queryClient } from "./queryClient.ts";
import { placeFromPath, useLastPlace } from "./servers/lastPlace.ts";
import { translate } from "./i18n/catalog.ts";
import { shell } from "./i18n/catalogs/shell.ts";
import { operator } from "./i18n/catalogs/operator.ts";
import {
  DEFAULT_OPERATOR_PAGE,
  isOperatorPage,
  validateOperatorSearch,
} from "./operator/areas.ts";

// Server settings are a chunk of their own. Not `lazyRouteComponent`: it
// reloads the page when a deploy removed the chunk, which ends a call.
const ServerSettings = lazyPart(
  () =>
    import("./pages/ServerSettingsPage.tsx").then(
      (module) => module.ServerSettingsPage,
    ),
  () => translate(shell, "lazyPart.serverSettingsFailed"),
);

// The operator area "Betrieb" is one chunk too, like the server settings.
const OperatorArea = lazyPart(
  () =>
    import("./operator/OperatorArea.tsx").then((module) => module.OperatorArea),
  () => translate(operator, "lazyPartFailed"),
);

const rootRoute = createRootRoute({
  component: AppShell,
});

export type AuthSearch = {
  redirect?: string;
  /** "expired": the session ended while the app was open. "deleted": the
   * account was just deleted (#519). */
  reason?: "expired" | "deleted";
  /** Carried from sign-in to "Passwort vergessen", so it is not retyped. */
  email?: string;
  /** A beta invitation's code (`/register?invitation=…`), which pre-fills
   * the register page's invitation field. */
  invitation?: string;
  /** Why a Google sign-in sent the visitor here instead of signing in. */
  oidc_error?: OidcError;
};

export const validateAuthSearch = (
  search: Record<string, unknown>,
): AuthSearch => {
  const out: AuthSearch = {};
  const target = safeRedirect(search.redirect);
  if (target) out.redirect = target;
  if (search.reason === "expired" || search.reason === "deleted")
    out.reason = search.reason;
  if (
    typeof search.email === "string" &&
    search.email.length > 0 &&
    search.email.length <= 254
  )
    out.email = search.email;
  // The router reads an all-digit code as a number.
  const invitation =
    typeof search.invitation === "number" &&
    Number.isSafeInteger(search.invitation)
      ? String(search.invitation)
      : search.invitation;
  if (typeof invitation === "string" && /^[A-Za-z0-9]{1,64}$/.test(invitation))
    out.invitation = invitation;
  if (isOidcError(search.oidc_error)) out.oidc_error = search.oidc_error;
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
    throw redirect({ href: signedInTarget(search), replace: true });
  }
}

/**
 * Where a signed-in visitor of login or register goes. A failed provider
 * sign-in started from Sicherheit ("Mit Google bestätigen") comes back to
 * `/login` while the tab is still signed in; its error goes on to Sicherheit,
 * which shows it.
 */
export function signedInTarget(search: AuthSearch): string {
  const target = search.redirect ?? "/";
  if (!search.oidc_error) return target;
  // Also from the Konto löschen dialog, which comes back with `?delete=1`.
  const [path, query] = target.split("?", 2);
  if (path !== SECURITY_PATH) return target;
  const params = new URLSearchParams(query);
  params.set("oidc_error", search.oidc_error);
  return `${path}?${params}`;
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
  // Open signed in too: expired reset links and the verify page link here,
  // and "Sicherheit" needs the current password.
  beforeLoad: () => ensureSession(),
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

// A first sign-in through a provider names its new account here; the
// token in the fragment works signed in or not, like a mailed link.
const signInNameRoute = createRoute({
  getParentRoute: () => authLayout,
  path: "/sign-in/name",
  beforeLoad: () => ensureSession(),
  component: SignInNamePage,
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
  component: () => (
    <ServerSettings.Component fallback={<MainSkeleton kind="list" />} />
  ),
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
    // Only Feedback waits: it exists while the instance takes feedback.
    if (params.area === "feedback") return requireFeedback();
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

/**
 * Guard for the operator area: anyone who is not an operator, and every
 * unknown page, gets "not found", so the area is not confirmed to exist.
 */
async function requireOperator(page?: string) {
  await ensureSession();
  if (!useSession.getState().operator) throw notFound();
  if (page !== undefined && !isOperatorPage(page)) throw notFound();
}

const operatorRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/betrieb",
  beforeLoad: async () => {
    await requireOperator();
    throw redirect({
      to: "/betrieb/$page",
      params: { page: DEFAULT_OPERATOR_PAGE },
      replace: true,
    });
  },
});

const operatorPageRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/betrieb/$page",
  validateSearch: validateOperatorSearch,
  beforeLoad: ({ params }) => requireOperator(params.page),
  component: function OperatorPageRoute() {
    const { page } = operatorPageRoute.useParams();
    const { q, entry } = operatorPageRoute.useSearch();
    return isOperatorPage(page) ? (
      <OperatorArea.Component
        page={page}
        q={q}
        entry={entry}
        fallback={<MainSkeleton kind="list" />}
      />
    ) : null;
  },
});

/** The directory "Entdecken" exists only while the instance runs one. */
async function requireDirectory() {
  await ensureSession();
  const stamp = takeStamp();
  if (!stamp) return;
  try {
    const status = await ensureDirectoryStatus(queryClient, stamp);
    if (!status.enabled) throw notFound();
  } catch (error) {
    // An API from before the directory has no status route.
    if (error instanceof ApiError && error.code === "not_found")
      throw notFound();
    throw error;
  }
}

/** Einstellungen → Feedback exists only while the instance takes feedback. */
async function requireFeedback() {
  await ensureSession();
  const stamp = takeStamp();
  if (!stamp) return;
  try {
    const status = await ensureFeedbackStatus(queryClient, stamp);
    if (!status.enabled) throw notFound();
  } catch (error) {
    // An API from before feedback has no status route.
    if (error instanceof ApiError && error.code === "not_found")
      throw notFound();
    throw error;
  }
}

const discoverRoute = createRoute({
  getParentRoute: () => workspaceLayout,
  path: "/discover",
  validateSearch: validateDiscoverSearch,
  beforeLoad: requireDirectory,
  component: function DiscoverRoute() {
    const { q, lang } = discoverRoute.useSearch();
    return <DiscoverPage q={q} lang={lang} />;
  },
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
    signInNameRoute,
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
    operatorRoute,
    operatorPageRoute,
    discoverRoute,
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
  defaultErrorComponent: RouteErrorPage,
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
