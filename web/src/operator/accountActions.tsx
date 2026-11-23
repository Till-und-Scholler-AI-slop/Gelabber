// What an operator can do to an account on the Konten page: write to it,
// disable it (a dialog with an optional reason) or enable it again, and for
// an address that was never confirmed, mail a new link or mark it confirmed
// (GEL-40). An operator's account cannot be disabled here: who is one is
// set by the deployment. The menus are data, so tests can choose their
// entries.

import { useState } from "react";

import { useSession } from "../auth/store.ts";
import type { MenuEntry } from "../components/rowMenuEntries.ts";
import { notify } from "../components/toasts.ts";
import { translate } from "../i18n/catalog.ts";
import { operator } from "../i18n/catalogs/operator.ts";
import * as remote from "./api.ts";
import { ReasonDialog, type ReasonRequest } from "./parts.tsx";
import { useOperatorOverview, useOperatorRefresh } from "./queries.ts";
import { askThenRun, useMessagePerson } from "./serverActions.tsx";
import type { Account } from "./types.ts";

export type AccountTarget = Pick<
  Account,
  "id" | "name" | "status" | "operator" | "email_verified"
>;

export type AccountHandlers = {
  /** Absent for your own account. */
  message?: () => void;
  disable: () => void;
  enable: () => void;
  /** Absent while the instance sends no mail. */
  resend?: () => void;
  confirm: () => void;
};

/**
 * A row's menu. An unconfirmed address that is not disabled can get a new
 * link, and be marked confirmed unless it is a pending registration. A disabled account can be enabled; any
 * other can be disabled unless it is an operator's. Empty when nothing
 * applies.
 */
export function accountMenuEntries(
  account: Pick<AccountTarget, "status" | "operator" | "email_verified">,
  on: AccountHandlers,
): MenuEntry[] {
  const entries: MenuEntry[] = [];
  if (on.message)
    entries.push({
      kind: "item",
      label: translate(operator, "accounts.menu.message"),
      onSelect: on.message,
    });
  if (!account.email_verified && account.status !== "disabled") {
    const help: MenuEntry[] = [];
    if (on.resend)
      help.push({
        kind: "item",
        label: translate(operator, "accounts.menu.resend"),
        onSelect: on.resend,
      });
    // Anyone who registers a pending address replaces its password, so
    // nobody can vouch for it; its owner gets a new link instead.
    if (account.status !== "pending")
      help.push({
        kind: "item",
        label: translate(operator, "accounts.menu.confirm"),
        onSelect: on.confirm,
      });
    if (help.length > 0 && entries.length > 0)
      entries.push({ kind: "separator" });
    entries.push(...help);
  }
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

export function resendConfirmation(
  account: AccountTarget,
  refresh: () => void,
) {
  const name = account.name;
  return askThenRun({
    title: translate(operator, "dialogs.resendTitle", { name }),
    message: translate(operator, "dialogs.resendMessage"),
    confirmLabel: translate(operator, "accounts.menu.resend"),
    tone: "neutral",
    run: () => remote.resendConfirmation(account.id),
    done: translate(operator, "toasts.resent", { name }),
    failed: translate(operator, "toasts.resendFailed"),
    gone: translate(operator, "toasts.accountGone"),
    refresh,
  });
}

/** Vouches for whoever chose the password, so the question says so. */
export function confirmAddress(account: AccountTarget, refresh: () => void) {
  const name = account.name;
  return askThenRun({
    title: translate(operator, "dialogs.confirmTitle", { name }),
    message: translate(operator, "dialogs.confirmMessage"),
    confirmLabel: translate(operator, "accounts.menu.confirm"),
    tone: "neutral",
    run: () => remote.confirmAccount(account.id),
    done: translate(operator, "toasts.confirmed", { name }),
    failed: translate(operator, "toasts.confirmFailed"),
    gone: translate(operator, "toasts.accountGone"),
    refresh,
  });
}

/** The account actions of the page plus the dialog; render `dialog` once. */
export function useAccountActions() {
  const me = useSession((state) => state.user?.id);
  const refresh = useOperatorRefresh();
  const message = useMessagePerson();
  const mail = useOperatorOverview().data?.mail === true;
  const [request, setRequest] = useState<ReasonRequest | null>(null);
  const handlers = (account: AccountTarget): AccountHandlers => ({
    message: account.id !== me ? () => message(account.id) : undefined,
    disable: () => setRequest(disableRequest(account, refresh)),
    enable: () => void enableAccount(account, refresh),
    resend: mail ? () => void resendConfirmation(account, refresh) : undefined,
    confirm: () => void confirmAddress(account, refresh),
  });
  return {
    handlers,
    dialog: <ReasonDialog request={request} onClose={() => setRequest(null)} />,
  };
}
