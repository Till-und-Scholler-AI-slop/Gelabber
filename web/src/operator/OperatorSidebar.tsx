import { Link } from "@tanstack/react-router";

import { UserPanel } from "../components/UserPanel.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { useT } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import { OPERATOR_PAGES } from "./areas.ts";
import { useOperatorOverview } from "./queries.ts";
import "./operator.css";

/** The workspace sidebar while the operator area is open. */
export function OperatorSidebar() {
  const t = useT(operator);
  const { data: overview } = useOperatorOverview();
  return (
    <aside
      className="channel-sidebar settings-sidebar"
      aria-label={t("sidebar.label")}
    >
      <div className="sidebar-brand">{t("title")}</div>
      <div className="settings-sidebar-scroll">
        <OperatorNavigation openReports={overview?.open_reports ?? 0} />
      </div>
      <UserPanel />
    </aside>
  );
}

export function OperatorNavigation({ openReports }: { openReports: number }) {
  const t = useT(operator);
  return (
    <nav className="settings-nav" aria-label={t("nav.label")}>
      <div className="settings-nav-group">
        <h2 className="settings-nav-heading" id="operator-nav-instance">
          {t("nav.instance")}
        </h2>
        <ul aria-labelledby="operator-nav-instance">
          {OPERATOR_PAGES.map((page) => (
            <li key={page}>
              <Link
                to="/betrieb/$page"
                params={{ page }}
                className="settings-nav-link"
              >
                <span>{t(`page.${page}.title`)}</span>
                {page === "meldungen" && openReports > 0 ? (
                  <>
                    <Badge
                      tone="count"
                      className="operator-nav-count"
                      aria-hidden="true"
                    >
                      {openReports > 99 ? "99+" : openReports}
                    </Badge>
                    <span className="sr-only">
                      {t("nav.reportsLabel", { count: openReports })}
                    </span>
                  </>
                ) : null}
              </Link>
            </li>
          ))}
        </ul>
      </div>
    </nav>
  );
}
