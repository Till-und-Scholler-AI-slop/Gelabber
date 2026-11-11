// URL-selected workspace. Queries and cache eviction retain their existing scope.
import { useQueryClient } from "@tanstack/react-query";
import {
  Outlet,
  useNavigate,
  useParams,
  useRouterState,
} from "@tanstack/react-router";
import {
  useEffect,
  useId,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";

import { ApiError } from "../api/client.ts";
import { useUserId } from "../auth/scope.ts";
import { useSession } from "../auth/session.ts";
import { ChannelSidebar } from "../components/ChannelSidebar.tsx";
import { DmSidebar } from "../components/DmSidebar.tsx";
import { MenuIcon } from "../components/Icons.tsx";
import { preloadPartsWhenIdle } from "../components/lazyPart.tsx";
import { QuickSwitcher, ShortcutSheet } from "../components/QuickSwitcher.tsx";
import { RequireUser } from "../components/RequireUser.tsx";
import { ServerRail } from "../components/ServerRail.tsx";
import { SidebarPlaceholder } from "../components/SidebarPlaceholder.tsx";
import { UserPanel } from "../components/UserPanel.tsx";
import {
  NAVIGATION_DRAWER_MAX_WIDTH,
  WorkspaceDrawer,
} from "../components/WorkspaceNavigation.tsx";
import { useEdgeSwipe } from "../components/edgeSwipe.ts";
import { tileInitials, tileTint } from "../components/initials.ts";
import { ServerIconFace } from "../components/ServerIcon.tsx";
import { useNavigationDrawer } from "../components/navigationDrawer.ts";
import { notify } from "../components/toasts.ts";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import { useShellShortcuts } from "../components/useShellShortcuts.ts";
import { useDms } from "../dms/queries.ts";
import { offerDesktopApp } from "../pwa/desktopApp.ts";
import { useInstallation } from "../pwa/install.ts";
import { useMuteClock } from "../messages/notificationSettings.ts";
import { useReadState, useReadingChannel } from "../messages/readState.ts";
import { unreadMark } from "../messages/unreadIndicators.ts";
import { forgetServer, useServer, useServers } from "../servers/queries.ts";
import { BlockedServerNotice } from "../servers/BlockedServerNotice.tsx";
import { SettingsSidebar } from "./UserSettingsPage.tsx";
import { OperatorSidebar } from "../operator/OperatorSidebar.tsx";
import { translate, useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

// After the first screen has settled, so it does not land among the start's
// own notices; not on the page it would lead to.
const DESKTOP_OFFER_DELAY_MS = 3000;

function useDesktopAppOffer(pathname: string) {
  const navigate = useNavigate();
  const platform = useInstallation((state) => state.desktop);
  const onAppSettings = pathname === "/settings/app";
  useEffect(() => {
    if (!platform || onAppSettings) return;
    const timer = setTimeout(
      () =>
        offerDesktopApp(
          platform,
          () =>
            void navigate({ to: "/settings/$area", params: { area: "app" } }),
        ),
      DESKTOP_OFFER_DELAY_MS,
    );
    return () => clearTimeout(timer);
  }, [platform, onAppSettings, navigate]);
}

export function WorkspaceLayout() {
  return (
    <RequireUser>
      <Workspace />
    </RequireUser>
  );
}

function Workspace() {
  const t = useT(shell);
  const { serverId, channelId } = useParams({ strict: false });
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const onDms = pathname === "/d" || pathname.startsWith("/d/");
  const onSettings =
    pathname === "/settings" || pathname.startsWith("/settings/");
  // Only for an operator: everyone else sees "not found" in the plain frame.
  const operator = useSession((state) => state.operator);
  const onOperator =
    operator && (pathname === "/betrieb" || pathname.startsWith("/betrieb/"));
  // Before the early returns so hooks keep their order; the rail shares it.
  const { data: servers } = useServers();
  // Voice room and settings load in the background once the first screen is
  // up, so a click does not wait for them and a deploy cannot take them away.
  useEffect(() => preloadPartsWhenIdle(), []);
  useDesktopAppOffer(pathname);

  if (serverId)
    return <SelectedServer serverId={serverId} channelId={channelId} />;
  if (onDms) return <SelectedDms channelId={channelId} />;
  if (onSettings)
    return (
      <WorkspaceFrame
        place={t("place.settings")}
        sidebar={<SettingsSidebar />}
      />
    );
  if (onOperator)
    return (
      <WorkspaceFrame
        place={t("place.operator")}
        sidebar={<OperatorSidebar />}
      />
    );
  return (
    <WorkspaceFrame
      place="Gelabber"
      sidebar={
        <aside className="channel-sidebar" aria-label={t("sidebar.label")}>
          <div className="sidebar-brand">Gelabber{/* i18n-ok */}</div>
          {/* Only once the list says so: never flash it at members. */}
          {servers?.length === 0 ? (
            <div className="sidebar-empty">{t("noServers")}</div>
          ) : null}
          <UserPanel />
        </aside>
      }
    />
  );
}

/** What the phone's top bar names: a server (its tile leads the drawer
 * button) or a plain label. */
type Place = string | { id: string; name: string; icon_url?: string | null };

function WorkspaceFrame({
  sidebar,
  place,
  serverId,
  dmActive = false,
  main,
}: {
  sidebar: ReactNode;
  place: Place;
  serverId?: string;
  dmActive?: boolean;
  /** Takes the place of the routed page (a blocked server's notice). */
  main?: ReactNode;
}) {
  const t = useT(shell);
  const drawer = useNavigationDrawer();
  const navigationId = useId();
  const { channelId } = useParams({ strict: false });
  const [switching, setSwitching] = useState(false);
  const [showingKeys, setShowingKeys] = useState(false);
  useShellShortcuts({
    serverId,
    channelId,
    dms: dmActive,
    onSwitcher: () => setSwitching(true),
    onHelp: () => setShowingKeys(true),
  });
  const phone = useMediaQuery(`(max-width: ${NAVIGATION_DRAWER_MAX_WIDTH}px)`);
  useEdgeSwipe({
    enabled: phone,
    open: drawer.open,
    drawerSelector: ".workspace-navigation",
    onOpen: drawer.show,
    onClose: drawer.hide,
  });
  // Phones keep every unread badge inside the closed drawer; its button
  // sums them up, with mutes and notification levels applied.
  const reading = useReadingChannel();
  const now = useMuteClock();
  const { data: readRows } = useReadState();
  const mark = unreadMark(readRows ?? [], reading, now);
  const count = mark.count > 99 ? "99+" : String(mark.count);
  const label =
    mark.count > 0
      ? mark.count > 99
        ? t("drawer.openLabelOver99")
        : t("drawer.openLabelCount", { count: mark.count })
      : mark.any
        ? t("drawer.openLabelUnread")
        : t("drawer.openLabel");
  return (
    <div className="workspace">
      <WorkspaceDrawer
        id={navigationId}
        open={drawer.open}
        onClose={drawer.hide}
        title={t("sidebar.label")}
        breakpoint={NAVIGATION_DRAWER_MAX_WIDTH}
        className="workspace-navigation"
      >
        <ServerRail activeId={serverId} dmActive={dmActive} />
        {sidebar}
      </WorkspaceDrawer>
      <div className="workspace-main">
        <header className="workspace-mobile-topbar">
          <button
            type="button"
            className="workspace-navigation-trigger"
            aria-label={label}
            aria-expanded={drawer.open}
            aria-controls={navigationId}
            onClick={drawer.show}
          >
            {typeof place === "string" ? (
              <MenuIcon size={22} />
            ) : (
              <span
                className="workspace-navigation-tile"
                style={{ "--server-tile": tileTint(place.id) } as CSSProperties}
                aria-hidden="true"
              >
                <ServerIconFace
                  url={place.icon_url}
                  fallback={tileInitials(place.name)}
                />
              </span>
            )}
            {mark.count > 0 ? (
              <span className="workspace-navigation-count" aria-hidden="true">
                {count}
              </span>
            ) : mark.any ? (
              <span
                className="workspace-navigation-unread"
                aria-hidden="true"
              />
            ) : null}
          </button>
          <span className="workspace-mobile-place">
            {typeof place === "string" ? place : place.name}
          </span>
        </header>
        <main
          id="workspace-content"
          tabIndex={-1}
          className="workspace-content"
        >
          {main ?? <Outlet />}
        </main>
      </div>
      <QuickSwitcher
        open={switching}
        onClose={() => setSwitching(false)}
        current={channelId}
      />
      <ShortcutSheet open={showingKeys} onClose={() => setShowingKeys(false)} />
    </div>
  );
}

function SelectedServer({
  serverId,
  channelId,
}: {
  serverId: string;
  channelId: string | undefined;
}) {
  const t = useT(shell);
  const client = useQueryClient();
  const navigate = useNavigate();
  const userId = useUserId();
  const { data: server, error, isFetching, refetch } = useServer(serverId);
  const { data: servers } = useServers();
  const listed = servers?.find((row) => row.id === serverId);
  const gone =
    error instanceof ApiError &&
    (error.code === "not_found" || error.code === "forbidden");
  // Blocked by an operator: the list says so, or a read just met it.
  const blockedRead =
    error instanceof ApiError && error.code === "server_blocked";
  const blocked = blockedRead || listed?.blocked === true;
  // Lifted meanwhile: the list (read again) knows first.
  const lifted = blockedRead && listed?.blocked === false;
  useEffect(() => {
    if (lifted) void refetch();
  }, [lifted, refetch]);

  useEffect(() => {
    if (!gone || !userId) return;
    notify(translate(shell, "serverGone"));
    forgetServer(client, userId, serverId, { keepDetail: true });
    void navigate({ to: "/", replace: true }).then(() =>
      forgetServer(client, userId, serverId),
    );
  }, [gone, client, navigate, serverId, userId]);

  if (gone) return null;
  if (blocked)
    return (
      <WorkspaceFrame
        serverId={serverId}
        place={listed ?? "Gelabber"}
        sidebar={
          <aside className="channel-sidebar" aria-label={t("sidebar.label")}>
            <div className="sidebar-brand">{listed?.name ?? "Gelabber"}</div>
            <UserPanel />
          </aside>
        }
        main={<BlockedServerNotice serverId={serverId} server={listed} />}
      />
    );
  return (
    <WorkspaceFrame
      serverId={serverId}
      place={server ?? listed ?? "Gelabber"}
      sidebar={
        server ? (
          <ChannelSidebar server={server} activeChannelId={channelId} />
        ) : (
          <SidebarPlaceholder
            failure={
              error
                ? {
                    message: t("channels.loadFailed"),
                    pending: isFetching,
                    onRetry: () => void refetch(),
                  }
                : undefined
            }
          />
        )
      }
    />
  );
}

function SelectedDms({ channelId }: { channelId: string | undefined }) {
  const t = useT(shell);
  const { data: dms, error, isFetching, refetch } = useDms();
  return (
    <WorkspaceFrame
      dmActive
      place={t("place.dms")}
      sidebar={
        dms ? (
          <DmSidebar dms={dms} activeChannelId={channelId} />
        ) : (
          <SidebarPlaceholder
            failure={
              error
                ? {
                    message: t("dmsLoadFailed"),
                    pending: isFetching,
                    onRetry: () => void refetch(),
                  }
                : undefined
            }
          />
        )
      }
    />
  );
}
