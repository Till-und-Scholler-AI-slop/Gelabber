// What an operator can do to an account on the Konten page: write to it,
// disable it (a dialog with an optional reason) or enable it again. An
// operator's account cannot be disabled here: who is one is set by the
// deployment. The menus are data, so tests can choose their entries.

import { useState } from "react";

import { useSession } from "../auth/store.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { notify } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import * as remote from "./api.ts";
import { ReasonDialog, type ReasonRequest } from "./parts.tsx";
import { useOperatorRefresh } from "./queries.ts";
import { askThenRun, useMessagePerson } from "./serverActions.tsx";
import type { Account } from "./types.ts";

export type AccountTarget = Pick<
  Account,
  "id" | "name" | "status" | "operator"
>;

export type AccountHandlers = {
  /** Absent for your own account. */
  message?: () => void;
  disable: () => void;
  enable: () => void;
};

/**
 * A row's menu. A disabled account can be enabled; any other can be
 * disabled unless it is an operator's. Empty when nothing applies.
 */
export function accountMenuEntries(
  account: Pick<AccountTarget, "status" | "operator">,
  on: AccountHandlers,
): MenuEntry[] {
  const entries: MenuEntry[] = [];
  if (on.message)
    entries.push({
      kind: "item",
      label: translate(operator, "accounts.menu.message"),
      onSelect: on.message,
    });
  const change: MenuEntry | null =
    account.status === "disabled"
      ? {
          kind: "item",
          label: translate(operator, "accounts.menu.enable"),
          onSelect: on.enable,
        }
      : account.operator
        ? null
        : {
            kind: "item",
            label: translate(operator, "accounts.menu.disable"),
            danger: true,
            onSelect: on.disable,
          };
  if (change) {
    if (entries.length > 0) entries.push({ kind: "separator" });
    entries.push(change);
  }
  return entries;
}

/** The dialog that disables an account: a reason is optional. */
export function disableRequest(
  account: AccountTarget,
  refresh: () => void,
): ReasonRequest {
  const name = account.name;
  return {
    title: translate(operator, "dialogs.disableTitle", { name }),
    message: translate(operator, "dialogs.disableMessage"),
    label: translate(operator, "dialogs.disableReason"),
    field: "reason",
    required: false,
    confirmLabel: translate(operator, "accounts.menu.disable"),
    danger: true,
    submit: async (reason) => {
      try {
        await remote.disableAccount(account.id, reason || undefined);
      } finally {
        refresh();
      }
      notify(translate(operator, "toasts.disabled", { name }));
    },
  };
}

export function enableAccount(account: AccountTarget, refresh: () => void) {
  const name = account.name;
  return askThenRun({
    title: translate(operator, "dialogs.enableTitle", { name }),
    message: translate(operator, "dialogs.enableMessage"),
    confirmLabel: translate(operator, "accounts.menu.enable"),
    tone: "neutral",
    run: () => remote.enableAccount(account.id),
    done: translate(operator, "toasts.enabled", { name }),
    failed: translate(operator, "toasts.enableFailed"),
    gone: translate(operator, "toasts.accountGone"),
    refresh,
  });
}

/** The account actions of the page plus the dialog; render `dialog` once. */
export function useAccountActions() {
  const me = useSession((state) => state.user?.id);
  const refresh = useOperatorRefresh();
  const message = useMessagePerson();
  const [request, setRequest] = useState<ReasonRequest | null>(null);
  const handlers = (account: AccountTarget): AccountHandlers => ({
    message: account.id !== me ? () => message(account.id) : undefined,
    disable: () => setRequest(disableRequest(account, refresh)),
    enable: () => void enableAccount(account, refresh),
  });
  return {
    handlers,
    dialog: <ReasonDialog request={request} onClose={() => setRequest(null)} />,
  };
}
