// "Konten": every account of the instance, newest first, with its servers
// and status, and what an operator can do to it.

import { useState } from "react";

import { Avatar } from "../../components/Avatar.tsx";
import { LoadError } from "../../components/LoadError.tsx";
import { useDebouncedValue } from "../../components/useDebouncedValue.ts";
import { Badge } from "../../components/ui/Badge.tsx";
import { ChipGroup } from "../../components/ui/ChipGroup.tsx";
import { SkeletonRows } from "../../components/ui/Skeleton.tsx";
import { useT } from "../../i18n/catalog.ts";
import { operator } from "../../i18n/catalogs/operator.ts";
import { formatDate } from "../../i18n/format.ts";
import { OperatorPageFrame } from "../OperatorPageFrame.tsx";
import { accountMenuEntries, useAccountActions } from "../accountActions.tsx";
import { formatCount } from "../format.ts";
import {
  Pager,
  RowMenuButton,
  SEARCH_DEBOUNCE_MS,
  SearchField,
} from "../parts.tsx";
import { usePaging } from "../paging.ts";
import { useOperatorAccounts } from "../queries.ts";
import type { Account, AccountFilter, AccountStatus } from "../types.ts";

const ACCOUNT_FILTERS = [
  "all",
  "disabled",
  "pending",
] as const satisfies readonly AccountFilter[];

const STATUS_TONE = {
  active: "success",
  pending: "warning",
  disabled: "danger",
} as const satisfies Record<AccountStatus, string>;

export function AccountsPage() {
  const t = useT(operator);
  const [input, setInput] = useState("");
  const search = useDebouncedValue(input.trim(), SEARCH_DEBOUNCE_MS);
  const [filter, setFilter] = useState<AccountFilter>("all");
  const [offset, setOffset] = usePaging(`${filter}\n${search}`);
  const query = useOperatorAccounts({
    q: search || undefined,
    filter,
    offset: offset || undefined,
  });
  const actions = useAccountActions();
  const data = query.data;

  return (
    <OperatorPageFrame page="konten">
      <div className="operator-toolbar">
        <SearchField
          label={t("accounts.searchLabel")}
          value={input}
          onChange={setInput}
        />
        <ChipGroup
          label={t("accounts.filterLabel")}
          options={ACCOUNT_FILTERS.map((value) => ({
            value,
            label: t(`accounts.filter.${value}`),
          }))}
          value={filter}
          onChange={setFilter}
        />
      </div>
      {query.isError && !data ? (
        <LoadError
          message={t("accounts.loadFailed")}
          pending={query.isFetching}
          onRetry={() => void query.refetch()}
        />
      ) : !data ? (
        <SkeletonRows count={4} />
      ) : data.accounts.length === 0 ? (
        <p className="operator-empty">{t("accounts.empty")}</p>
      ) : (
        <>
          <AccountTable
            accounts={data.accounts}
            menu={(account) =>
              accountMenuEntries(account, actions.handlers(account))
            }
          />
          <Pager offset={offset} total={data.total} onOffset={setOffset} />
        </>
      )}
      {actions.dialog}
    </OperatorPageFrame>
  );
}

export function AccountTable({
  accounts,
  menu,
}: {
  accounts: Account[];
  menu: (account: Account) => ReturnType<typeof accountMenuEntries>;
}) {
  const t = useT(operator);
  const column = {
    email: t("accounts.column.email"),
    since: t("accounts.column.since"),
    servers: t("accounts.column.servers"),
    status: t("accounts.column.status"),
  };
  return (
    <table className="operator-table operator-table--accounts">
      <thead>
        <tr>
          <th scope="col">{t("accounts.column.name")}</th>
          <th scope="col">{column.email}</th>
          <th scope="col">{column.since}</th>
          <th
            scope="col"
            className="is-number"
            title={t("accounts.column.serversHint")}
          >
            {column.servers}
          </th>
          <th scope="col">{column.status}</th>
          <th scope="col" className="is-menu">
            <span className="sr-only">{t("accounts.column.actions")}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        {accounts.map((account) => {
          const entries = menu(account);
          return (
            <tr key={account.id}>
              <th scope="row" className="is-title">
                <span className="operator-person">
                  <Avatar
                    name={account.name}
                    url={account.avatar_url}
                    decorative
                  />
                  <span className="operator-server-name">{account.name}</span>
                  {account.operator ? (
                    <Badge tone="accent">{t("accounts.operator")}</Badge>
                  ) : null}
                </span>
              </th>
              <td data-label={column.email} className="is-email">
                {account.email}
              </td>
              <td data-label={column.since}>
                <time dateTime={account.created_at}>
                  {formatDate(account.created_at, { dateStyle: "medium" })}
                </time>
              </td>
              <td data-label={column.servers} className="is-number">
                <span aria-hidden="true">
                  {t("accounts.servers", {
                    owned: formatCount(account.owned_servers),
                    member: formatCount(account.member_servers),
                  })}
                </span>
                <span className="sr-only">
                  {t("accounts.serversLabel", {
                    owned: formatCount(account.owned_servers),
                    member: formatCount(account.member_servers),
                  })}
                </span>
              </td>
              <td data-label={column.status}>
                <Badge
                  tone={STATUS_TONE[account.status]}
                  title={
                    account.disabled_reason
                      ? t("accounts.disabledReason", {
                          reason: account.disabled_reason,
                        })
                      : undefined
                  }
                >
                  {t(`accounts.status.${account.status}`)}
                </Badge>
              </td>
              <td className="is-menu">
                {entries.length > 0 ? (
                  <RowMenuButton
                    label={t("row.menuLabel", { name: account.name })}
                    entries={entries}
                  />
                ) : null}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
