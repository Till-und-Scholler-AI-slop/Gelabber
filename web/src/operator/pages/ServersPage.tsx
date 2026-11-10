// "Server": every server of the instance with its owner, size, place in
// the directory and open reports, and what an operator can do to it
// (reference docs/design/v0.10/hosted-operator-console.png).

import { useState } from "react";

import { LoadError } from "../../components/LoadError.tsx";
import { useDebouncedValue } from "../../components/useDebouncedValue.ts";
import { Badge } from "../../components/ui/Badge.tsx";
import { ChipGroup } from "../../components/ui/ChipGroup.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { useT } from "../../i18n/catalog.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import { formatBytes, formatCount } from "../format.ts";
import {
  Pager,
  RowMenuButton,
  SEARCH_DEBOUNCE_MS,
  SearchField,
  ServerName,
} from "../parts.tsx";
import { usePaging } from "../paging.ts";
import { useOperatorServers } from "../queries.ts";
import { serverMenuEntries, useServerActions } from "../serverActions.tsx";
import type { ServerAdmin, ServerFilter } from "../types.ts";

const SERVER_FILTERS = [
  "all",
  "listed",
  "reported",
  "blocked",
] as const satisfies readonly ServerFilter[];

export function ServersPage({ q }: { q?: string }) {
  const t = useT(operator);
  const [input, setInput] = useState(q ?? "");
  const search = useDebouncedValue(input.trim(), SEARCH_DEBOUNCE_MS);
  const [filter, setFilter] = useState<ServerFilter>("all");
  const [offset, setOffset, settle] = usePaging(`${filter}\n${search}`);
  const query = useOperatorServers({
    q: search || undefined,
    filter,
    offset: offset || undefined,
  });
  const actions = useServerActions();
  const data = query.data;
  settle(
    data && !query.isPlaceholderData
      ? { rows: data.servers.length, total: data.total }
      : undefined,
  );

  return (
    <OperatorPageFrame page="server">
      <div className="operator-toolbar">
        <SearchField
          label={t("servers.searchLabel")}
          value={input}
          onChange={setInput}
        />
        <ChipGroup
          label={t("servers.filterLabel")}
          options={SERVER_FILTERS.map((value) => ({
            value,
            label: t(`servers.filter.${value}`),
          }))}
          value={filter}
          onChange={setFilter}
        />
      </div>
      {query.isError && !data ? (
        <LoadError
          message={t("servers.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={4} />
      ) : data.servers.length === 0 ? (
        <p className="operator-empty">{t("servers.empty")}</p>
      ) : (
        <>
          <ServerTable
            servers={data.servers}
            menu={(server) => {
              const target = {
                id: server.id,
                name: server.name,
                directory: server.directory,
                blocked: server.blocked_at !== null,
              };
              return serverMenuEntries(
                target,
                actions.handlers(target, server.owner.id),
              );
            }}
          />
          <Pager offset={offset} total={data.total} onOffset={setOffset} />
        </>
      )}
      <p className="operator-footnote">{t("servers.footnote")}</p>
      {actions.dialog}
    </OperatorPageFrame>
  );
}

export function ServerTable({
  servers,
  menu,
}: {
  servers: ServerAdmin[];
  menu: (server: ServerAdmin) => ReturnType<typeof serverMenuEntries>;
}) {
  const t = useT(operator);
  const column = {
    owner: t("servers.column.owner"),
    members: t("servers.column.members"),
    storage: t("servers.column.storage"),
    directory: t("servers.column.directory"),
    reports: t("servers.column.reports"),
  };
  return (
    <table className="operator-table operator-table--servers">
      <thead>
        <tr>
          <th scope="col">{t("servers.column.server")}</th>
          <th scope="col">{column.owner}</th>
          <th scope="col" className="is-number">
            {column.members}
          </th>
          <th scope="col" className="is-number">
            {column.storage}
          </th>
          <th scope="col">{column.directory}</th>
          <th scope="col" className="is-number">
            {column.reports}
          </th>
          <th scope="col" className="is-menu">
            <span className="sr-only">{t("servers.column.actions")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {servers.map((server) => (
          <tr key={server.id}>
            <th scope="row" className="is-title">
              <ServerName
                server={server}
                badge={
                  server.blocked_at ? (
                    <Badge tone="danger">{t("servers.blocked")}</Badge>
                  ) : null
                }
              />
            </th>
            <td data-label={column.owner} title={server.owner.email}>
              {server.owner.name}
            </td>
            <td data-label={column.members} className="is-number">
              {formatCount(server.members)}
            </td>
            <td data-label={column.storage} className="is-number">
              {formatBytes(server.storage_bytes)}
            </td>
            <td data-label={column.directory}>
              <DirectoryCell directory={server.directory} />
            </td>
            <td data-label={column.reports} className="is-number">
              <ReportCount count={server.open_reports} />
            </td>
            <td className="is-menu">
              <RowMenuButton
                label={t("row.menuLabel", { name: server.name })}
                entries={menu(server)}
              />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function DirectoryCell({ directory }: { directory: ServerAdmin["directory"] }) {
  const t = useT(operator);
  if (directory === "off")
    return (
      <>
        <span aria-hidden="true">{t("cell.none")}</span>
        <span className="sr-only">{t("servers.directory.offLabel")}</span>
      </>
    );
  return <Badge>{t(`servers.directory.${directory}`)}</Badge>;
}

/** Red when there is anything open, as in the reference. */
function ReportCount({ count }: { count: number }) {
  const t = useT(operator);
  if (count === 0) return <>{formatCount(0)}</>;
  return (
    <>
      <Badge tone="count" className="operator-count" aria-hidden="true">
        {count > 99 ? "99+" : formatCount(count)}
      </Badge>
      <span className="sr-only">{t("nav.reportsLabel", { count })}</span>
    </>
  );
}
