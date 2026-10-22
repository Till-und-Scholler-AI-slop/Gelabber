// `/settings` and `/settings/$area`: personal settings inside the workspace.
// The sidebar lists the areas; on phones `/settings` is that list and each
// area a screen of its own with a way back.

import {
  Link,
  Navigate,
  useCanGoBack,
  useLocation,
  useNavigate,
  useParams,
  useRouter,
} from "@tanstack/react-router";
import { useEffect, useRef } from "react";

import { ChevronIcon } from "../components/Icons.tsx";
import { ScopeTag } from "../components/SettingsGroup.tsx";
import { IconButton } from "../components/ui/IconButton.tsx";
import { SkeletonRows } from "../components/ui/Skeleton.tsx";
import { useMediaQuery } from "../components/useMediaQuery.ts";
import { UserPanel } from "../components/UserPanel.tsx";
import { useInstallation } from "../pwa/install.ts";
import {
  DEFAULT_SETTINGS_AREA,
  isSettingsArea,
  SETTINGS_GROUPS,
  SETTINGS_LIST_QUERY,
  settingsArea,
  visibleSettingsAreas,
  type SettingsArea,
} from "./settingsAreas.ts";
import { SettingsAreaPart } from "./settingsAreaPart.ts";
import "./settings.css";

function useInstallable() {
  // Nothing to install inside the desktop app or the installed web app.
  return useInstallation((state) => !state.native && !state.standalone);
}

/** The areas, grouped "Konto" and "App". `list` draws them as the phone's
 * list screen: taller rows with a chevron. */
function SettingsNavigation({
  current,
  list = false,
}: {
  current?: SettingsArea | undefined;
  list?: boolean;
}) {
  const areas = visibleSettingsAreas(useInstallable(), current);
  return (
    <nav
      className={list ? "settings-nav settings-nav--list" : "settings-nav"}
      aria-label="Einstellungsbereiche"
    >
      {SETTINGS_GROUPS.map((group) => (
        <div key={group.id} className="settings-nav-group">
          <h2 className="settings-nav-heading" id={`settings-nav-${group.id}`}>
            {group.label}
          </h2>
          <ul aria-labelledby={`settings-nav-${group.id}`}>
            {areas
              .filter((area) => area.group === group.id)
              .map((area) => (
                <li key={area.id}>
                  <Link
                    to="/settings/$area"
                    params={{ area: area.id }}
                    state={list ? { settingsList: true } : undefined}
                    className="settings-nav-link"
                  >
                    <span>{area.label}</span>
                    {list ? <ChevronIcon size={18} /> : null}
                  </Link>
                </li>
              ))}
          </ul>
        </div>
      ))}
    </nav>
  );
}

/** The workspace sidebar while settings are open. */
export function SettingsSidebar() {
  const params = useParams({ strict: false });
  const current = isSettingsArea(params.area) ? params.area : undefined;
  return (
    <aside className="channel-sidebar settings-sidebar" aria-label="Navigation">
      <div className="sidebar-brand">Einstellungen</div>
      <div className="settings-sidebar-scroll">
        <SettingsNavigation current={current} />
      </div>
      <UserPanel />
    </aside>
  );
}

/** `/settings` on a phone: the list of areas. Wider screens go on to the
 * default area, whose sidebar already is this list. */
export function SettingsListPage() {
  const listFirst = useMediaQuery(SETTINGS_LIST_QUERY);
  if (!listFirst)
    return (
      <Navigate
        to="/settings/$area"
        params={{ area: DEFAULT_SETTINGS_AREA }}
        replace
      />
    );
  return (
    <div className="settings-screen">
      <header className="lr-channel-header settings-header">
        <h1>Einstellungen</h1>
      </header>
      <div className="settings-scroll">
        <SettingsNavigation list />
      </div>
    </div>
  );
}

function BackToList() {
  const router = useRouter();
  const canGoBack = useCanGoBack();
  const fromList = useLocation({ select: (l) => l.state.settingsList });
  const navigate = useNavigate();
  return (
    <IconButton
      className="settings-back"
      label="Alle Einstellungen"
      icon={<ChevronIcon size={20} />}
      onClick={() => {
        // Opened from the list: going back keeps the history as it was.
        if (fromList && canGoBack) router.history.back();
        else void navigate({ to: "/settings" });
      }}
    />
  );
}

export function UserSettingsPage() {
  const { area } = useParams({ from: "/workspace/settings/$area" });
  // The route only matches known areas.
  const current = isSettingsArea(area) ? area : DEFAULT_SETTINGS_AREA;
  const info = settingsArea(current);
  const listFirst = useMediaQuery(SETTINGS_LIST_QUERY);
  const heading = useRef<HTMLHeadingElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  // A new area is a new page: start at its top, and move focus to its
  // heading so screen readers say where they are now.
  useEffect(() => {
    scroller.current?.scrollTo({ top: 0 });
    heading.current?.focus({ preventScroll: true });
  }, [current]);

  return (
    <div className="settings-screen">
      <header className="lr-channel-header settings-header">
        {listFirst ? <BackToList /> : null}
        <h1>Einstellungen</h1>
      </header>
      <div ref={scroller} className="settings-scroll">
        <section
          className={`settings-area settings-area--${current}`}
          aria-labelledby="settings-area-heading"
        >
          <header className="settings-area-heading">
            <div className="settings-area-title">
              <h2 id="settings-area-heading" ref={heading} tabIndex={-1}>
                {info.label}
              </h2>
              {"scope" in info ? <ScopeTag scope={info.scope} /> : null}
            </div>
            <p>{info.description}</p>
          </header>
          <SettingsAreaPart.Component
            area={current}
            fallback={<SkeletonRows count={4} />}
          />
        </section>
      </div>
    </div>
  );
}
