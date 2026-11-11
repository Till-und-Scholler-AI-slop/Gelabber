// "Auslastung": exactly the numbers `GET /api/operator/usage` returns,
// grouped, each with a line saying what it counts. No charts, no history:
// the API keeps none. Media-server numbers stay in Prometheus and Grafana.

import { LoadError } from "../../components/LoadError.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { useT } from "../../i18n/catalog.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { formatDate } from "../../i18n/format.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import { useOperatorUsage } from "../queries.ts";
import { usageGroups } from "../usage.ts";

export function UsagePage() {
  const t = useT(operator);
  const query = useOperatorUsage();
  const data = query.data;

  return (
    <OperatorPageFrame page="auslastung">
      {query.isError && !data ? (
        <LoadError
          message={t("usage.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={5} />
      ) : (
        <>
          <p className="operator-notice">
            <strong>{t(`invitations.mode.${data.registration}`)}</strong>
            <span className="operator-muted">
              {" · "}
              {t("usage.generatedAt", {
                time: formatDate(data.generated_at, {
                  dateStyle: "medium",
                  timeStyle: "short",
                }),
              })}
            </span>
          </p>
          {usageGroups(data).map((group) => (
            <section
              key={group.key}
              className="operator-stats"
              aria-labelledby={`operator-usage-${group.key}`}
            >
              <h2 id={`operator-usage-${group.key}`}>{group.title}</h2>
              <dl>
                {group.stats.map((stat) => (
                  <div key={stat.key} className="operator-stat">
                    <dt>{stat.label}</dt>
                    <dd>{stat.value}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </>
      )}
      <p className="operator-footnote">{t("usage.footnote")}</p>
    </OperatorPageFrame>
  );
}
