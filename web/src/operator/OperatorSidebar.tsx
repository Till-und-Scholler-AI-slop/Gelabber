import { Link } from "@tanstack/react-router";

import { UserPanel } from "../components/UserPanel.tsx";
import { Badge } from "../components/ui/Badge.tsx";
import { useT } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import { OPERATOR_PAGES, type OperatorPage } from "./areas.ts";
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
        <OperatorNavigation
          openReports={overview?.open_reports ?? 0}
          openFeedback={overview?.open_feedback ?? 0}
        />
      </div>
      <UserPanel />
    </aside>
  );
}

export function OperatorNavigation({
  openReports,
  openFeedback,
}: {
  openReports: number;
  openFeedback: number;
}) {
  const t = useT(operator);
  // The pages that count what waits for an operator, and how they say it.
  const waiting = (page: OperatorPage): [number, string] =>
    page === "meldungen"
      ? [openReports, t("nav.reportsLabel", { count: openReports })]
      : page === "feedback"
        ? [openFeedback, t("nav.feedbackLabel", { count: openFeedback })]
        : [0, ""];
  return (
    <nav className="settings-nav" aria-label={t("nav.label")}>
      <div className="settings-nav-group">
        <h2 className="settings-nav-heading" id="operator-nav-instance">
          {t("nav.instance")}
        </h2>
        <ul aria-labelledby="operator-nav-instance">
          {OPERATOR_PAGES.map((page) => {
            const [count, label] = waiting(page);
            return (
              <li key={page}>
                <Link
                  to="/betrieb/$page"
                  params={{ page }}
                  className="settings-nav-link"
                >
                  <span>{t(`page.${page}.title`)}</span>
                  {count > 0 ? (
                    <>
                      <Badge
                        tone="count"
                        className="operator-nav-count"
                        aria-hidden="true"
                      >
                        {count > 99 ? "99+" : count}
                      </Badge>
                      <span className="sr-only">{label}</span>
                    </>
                  ) : null}
                </Link>
              </li>
            );
          })}
        </ul>
      </div>
    </nav>
  );
}
