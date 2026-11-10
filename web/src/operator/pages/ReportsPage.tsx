// "Meldungen": reported servers, the open ones oldest first. A report is
// closed without action here, or by taking its server out of the directory
// or blocking it, which closes all of the server's open reports.

import { useNavigate } from "@tanstack/react-router";
import { useLayoutEffect, useRef, useState } from "react";

import { LoadError } from "../../components/LoadError.tsx";
import { Badge } from "../../components/ui/Badge.tsx";
import { ChipGroup } from "../../components/ui/ChipGroup.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { useT } from "../../i18n/catalog.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { formatDate } from "../../i18n/format.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import { formatWhen } from "../format.ts";
import { usePaging } from "../paging.ts";
import { Pager, RowMenuButton, ServerName } from "../parts.tsx";
import { useOperatorReports } from "../queries.ts";
import {
  dismissRequest,
  reportMenuEntries,
  useServerActions,
  type ReportHandlers,
} from "../serverActions.tsx";
import type { Report, ReportListStatus } from "../types.ts";

const REPORT_FILTERS = [
  "open",
  "closed",
] as const satisfies readonly ReportListStatus[];

export function ReportsPage() {
  const t = useT(operator);
  const navigate = useNavigate();
  const [status, setStatus] = useState<ReportListStatus>("open");
  const [offset, setOffset, settle] = usePaging(status);
  const query = useOperatorReports({ status, offset: offset || undefined });
  const actions = useServerActions();
  const data = query.data;
  settle(
    data && !query.isPlaceholderData
      ? { rows: data.reports.length, total: data.total }
      : undefined,
  );

  const handlers = (report: Report): ReportHandlers => {
    const target = {
      id: report.server.id,
      name: report.server.name,
      directory: report.server.directory,
      blocked: report.server.blocked,
    };
    const server = actions.handlers(target);
    return {
      open: () =>
        void navigate({
          to: "/betrieb/$page",
          params: { page: "server" },
          search: { q: report.server.name },
        }),
      dismiss: () => actions.ask(dismissRequest(report, actions.refresh)),
      delist: server.delist,
      block: server.block,
    };
  };

  return (
    <OperatorPageFrame page="meldungen">
      <div className="operator-toolbar">
        <ChipGroup
          label={t("reports.filterLabel")}
          options={REPORT_FILTERS.map((value) => ({
            value,
            label: t(`reports.filter.${value}`),
          }))}
          value={status}
          onChange={setStatus}
        />
      </div>
      {query.isError && !data ? (
        <LoadError
          message={t("reports.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={4} />
      ) : data.reports.length === 0 ? (
        <p className="operator-empty">{t(`reports.empty.${status}`)}</p>
      ) : (
        <>
          <ReportTable
            reports={data.reports}
            menu={(report) => reportMenuEntries(report, handlers(report))}
          />
          <Pager offset={offset} total={data.total} onOffset={setOffset} />
        </>
      )}
      {actions.dialog}
    </OperatorPageFrame>
  );
}

export function ReportTable({
  reports,
  menu,
}: {
  reports: Report[];
  menu: (report: Report) => ReturnType<typeof reportMenuEntries>;
}) {
  const t = useT(operator);
  const column = {
    reason: t("reports.column.reason"),
    details: t("reports.column.details"),
    reporter: t("reports.column.reporter"),
    when: t("reports.column.when"),
  };
  return (
    <table className="operator-table operator-table--reports">
      <thead>
        <tr>
          <th scope="col">{t("reports.column.server")}</th>
          <th scope="col">{column.reason}</th>
          <th scope="col">{column.details}</th>
          <th scope="col">{column.reporter}</th>
          <th scope="col">{column.when}</th>
          <th scope="col" className="is-menu">
            <span className="sr-only">{t("reports.column.actions")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {reports.map((report) => (
          <tr key={report.id}>
            <th scope="row" className="is-title">
              <ServerName
                server={report.server}
                badge={
                  report.server.blocked ? (
                    <Badge tone="danger">{t("servers.blocked")}</Badge>
                  ) : null
                }
              />
            </th>
            <td data-label={column.reason}>
              {t(`reports.reason.${report.reason}`)}
            </td>
            <td data-label={column.details} className="is-details">
              <Details text={report.details} />
            </td>
            <td data-label={column.reporter}>
              {report.reporter?.name ?? (
                <span className="operator-muted">
                  {t("reports.reporterGone")}
                </span>
              )}
            </td>
            <td data-label={column.when}>
              <time
                dateTime={report.created_at}
                title={formatDate(report.created_at, {
                  dateStyle: "medium",
                  timeStyle: "short",
                })}
              >
                {formatWhen(report.created_at)}
              </time>
              {report.status === "open" ? null : (
                <Resolution report={report} outcome={report.status} />
              )}
            </td>
            <td className="is-menu">
              <RowMenuButton
                label={t("reports.menuLabel", { name: report.server.name })}
                entries={menu(report)}
              />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** How a closed report was closed: outcome, by whom, when, and the note. */
function Resolution({
  report,
  outcome,
}: {
  report: Report;
  outcome: "dismissed" | "actioned";
}) {
  const t = useT(operator);
  return (
    <div className="operator-resolution">
      <Badge tone={outcome === "actioned" ? "warning" : "neutral"}>
        {t(`reports.outcome.${outcome}`)}
      </Badge>
      <span className="operator-muted">
        {" "}
        {report.resolved_by?.name ?? t("reports.resolverGone")}
        {report.resolved_at ? (
          <>
            {" · "}
            <time
              dateTime={report.resolved_at}
              title={formatDate(report.resolved_at, {
                dateStyle: "medium",
                timeStyle: "short",
              })}
            >
              {formatWhen(report.resolved_at)}
            </time>
          </>
        ) : null}
      </span>
      {report.resolution_note ? (
        <p className="operator-muted">{report.resolution_note}</p>
      ) : null}
    </div>
  );
}

/** Two lines, and the rest behind "Mehr anzeigen" when there is more. */
function Details({ text }: { text: string }) {
  const t = useT(operator);
  const box = useRef<HTMLParagraphElement>(null);
  const [open, setOpen] = useState(false);
  const [overflows, setOverflows] = useState(false);
  useLayoutEffect(() => {
    const element = box.current;
    if (element && !open)
      setOverflows(element.scrollHeight > element.clientHeight + 1);
  }, [text, open]);
  if (!text.trim())
    return (
      <>
        <span aria-hidden="true">{t("cell.none")}</span>
        <span className="sr-only">{t("cell.noneLabel")}</span>
      </>
    );
  return (
    <>
      <p ref={box} className={`operator-details${open ? " is-open" : ""}`}>
        {text}
      </p>
      {overflows || open ? (
        <button
          type="button"
          className="operator-more"
          aria-expanded={open}
          onClick={() => setOpen((value) => !value)}
        >
          {open ? t("reports.less") : t("reports.more")}
        </button>
      ) : null}
    </>
  );
}
